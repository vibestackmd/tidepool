//! `fetch_and_cache_asset` — the uncompressed-asset hot path.
//!
//! Flow:
//! 1. Upstream `get_account(address)`.
//! 2. If the account's owner is SPL Token / Token-2022, treat the
//!    address as a mint — derive the Metaplex Metadata PDA and
//!    refetch. This is the "mint-as-id" routing Helius does for
//!    legacy-NFT queries where users pass a mint address, not the
//!    Metadata PDA.
//! 3. Dispatch to the first registered decoder whose `program_id()`
//!    matches the account's owner.
//! 4. Populate the cache on success.
//!
//! Owner-resolution for Token Metadata (scanning the mint's token
//! accounts to find the holder) is not yet implemented here — it
//! requires `getProgramAccounts` + a memcmp filter dispatched through
//! the upstream's generic `rpc_call`. Deferred to a follow-up; the
//! Token Metadata decoder leaves `ownership.owner` blank, and
//! by-owner indexing skips blank-owner entries in the cache layer so
//! partially-populated assets don't pollute query results.

use std::str::FromStr;
use std::sync::Arc;

use solana_program::pubkey::Pubkey;

use crate::cache::CacheStore;
use crate::das::{AccountDecoder, DasAsset};
use crate::upstream::{AccountData, UpstreamClient};

/// Well-known Solana program IDs we recognize as "mint containers"
/// for the mint-as-id routing pass.
const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const SPL_TOKEN_MIN_MINT_SIZE: usize = 82;
const MPL_TOKEN_METADATA_PROGRAM: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error(transparent)]
    Upstream(#[from] crate::upstream::UpstreamError),
    #[error(transparent)]
    Decoder(#[from] crate::das::DecoderError),
    #[error(transparent)]
    Cache(#[from] crate::cache::CacheError),
}

pub type FetchResult<T> = Result<T, FetchError>;

/// Fetch an uncompressed asset, decode it, cache it. Returns
/// `Ok(None)` when the address has no backing account (or when no
/// registered decoder matches the account's owner); `Ok(Some(asset))`
/// on success.
pub async fn fetch_and_cache_asset<U, C>(
    upstream: &U,
    cache: &C,
    decoders: &[Arc<dyn AccountDecoder>],
    address: &str,
) -> FetchResult<Option<DasAsset>>
where
    U: UpstreamClient + ?Sized,
    C: CacheStore + ?Sized,
{
    let Some(mut account) = upstream.get_account(address).await? else {
        return Ok(None);
    };

    // Mint-as-id routing: if the fetched account belongs to SPL Token
    // or Token-2022 and is at least a mint's size, treat `address` as
    // a mint and redirect to its Metaplex Metadata PDA. We reassign
    // `account` to the Metadata PDA's data so the decoder dispatch
    // below sees the Metadata-owned account.
    let owner_str = bs58::encode(account.owner).into_string();
    if (owner_str == SPL_TOKEN_PROGRAM_ID || owner_str == TOKEN_2022_PROGRAM_ID)
        && account.data.len() >= SPL_TOKEN_MIN_MINT_SIZE
    {
        let metadata_pda = derive_metadata_pda(address);
        let Some(md_account) = upstream.get_account(&metadata_pda).await? else {
            return Ok(None);
        };
        account = md_account;
    }

    let owner_str = bs58::encode(account.owner).into_string();
    for decoder in decoders {
        if decoder.program_id() == owner_str {
            if let Some(asset) = decoder.decode(address, &account.data)? {
                cache.put_asset(asset.clone()).await?;
                return Ok(Some(asset));
            }
        }
    }

    Ok(None)
}

/// Bypass for cases where the caller already has the raw account —
/// skips the upstream round-trip but still runs decoder dispatch +
/// cache populate. Useful for testing and for batch flows that
/// pre-fetch multiple accounts in one RPC call.
pub async fn decode_and_cache<C>(
    cache: &C,
    decoders: &[Arc<dyn AccountDecoder>],
    address: &str,
    account: &AccountData,
) -> FetchResult<Option<DasAsset>>
where
    C: CacheStore + ?Sized,
{
    let owner_str = bs58::encode(account.owner).into_string();
    for decoder in decoders {
        if decoder.program_id() == owner_str {
            if let Some(asset) = decoder.decode(address, &account.data)? {
                cache.put_asset(asset.clone()).await?;
                return Ok(Some(asset));
            }
        }
    }
    Ok(None)
}

/// PDA derivation for the Metaplex Metadata account:
/// `find_program_address(&[b"metadata", PROGRAM_ID, mint], PROGRAM_ID)`.
/// Inline rather than calling `mpl_token_metadata::Metadata::find_pda`
/// because that crate's `Pubkey` type is a different solana-program
/// major version than ours; crossing the bridge costs us more lines
/// than the ~10-line inline derivation.
fn derive_metadata_pda(mint_b58: &str) -> String {
    let Ok(mint) = Pubkey::from_str(mint_b58) else {
        return String::new();
    };
    let Ok(program) = Pubkey::from_str(MPL_TOKEN_METADATA_PROGRAM) else {
        return String::new();
    };
    let (pda, _bump) = Pubkey::find_program_address(
        &[b"metadata", program.as_ref(), mint.as_ref()],
        &program,
    );
    pda.to_string()
}
