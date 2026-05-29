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
//! For Token Metadata (`V1_NFT`), the decoder leaves `ownership.owner`
//! blank because the Metadata account doesn't carry the holding wallet
//! — only the mint. We resolve the holder here by calling
//! `getTokenLargestAccounts(mint)` (top holder of an NFT is by
//! definition the sole owner, since supply=1) and then reading the
//! token account's 32-byte owner field. Two extra RPC round-trips, but
//! only on the uncompressed-asset hot path — cNFTs and MplCore don't
//! need this.

use std::str::FromStr;
use std::sync::Arc;

use mpl_token_metadata::accounts::{Edition, MasterEdition, Metadata};
use mpl_token_metadata::types::Key;
use solana_program::pubkey::Pubkey;

use crate::cache::CacheStore;
use crate::das::{AccountDecoder, DasAsset, MasterEditionRecord, PrintEditionRecord};
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
            if let Some(mut asset) = decoder.decode(address, &account.data)? {
                // Token Metadata decoders emit assets with an empty
                // owner because the Metadata account doesn't carry
                // the holding wallet — only the mint does. Resolve
                // via getTokenLargestAccounts regardless of which
                // interface string the decoder produced (V1_NFT,
                // ProgrammableNFT, FungibleAsset, FungibleToken all
                // share this constraint).
                if asset.ownership.owner.is_empty() {
                    if let Some(owner) = resolve_token_metadata_owner(upstream, &account).await {
                        asset.ownership.owner = owner;
                    }
                }
                // Fold off-chain JSON (image/description/attributes/
                // files) into the asset before caching. Fail-soft: a
                // blocked or slow fetch leaves the on-chain fields and
                // returns quietly. Cached as part of the asset, so the
                // fetch happens once per asset.
                crate::das::enrich_offchain_metadata(upstream, &mut asset).await;
                cache.put_asset(asset.clone()).await?;
                // Token Metadata side-effect: index the mint's Edition
                // PDA (if any) so `getNftEditions` can serve it later.
                // Best-effort; failures don't break the primary fetch.
                if matches!(
                    asset.interface.as_str(),
                    "V1_NFT" | "ProgrammableNFT" | "LegacyNFT"
                ) {
                    index_edition_pda(upstream, cache, address).await;
                }
                return Ok(Some(asset));
            }
        }
    }

    Ok(None)
}

/// Fetch the Metaplex Edition PDA for `mint` and, if it exists, record
/// the master-vs-print relationship in the cache. Silent on every
/// failure path — `getNftEditions` just returns empty for masters we
/// never managed to index.
async fn index_edition_pda<U, C>(upstream: &U, cache: &C, mint_b58: &str)
where
    U: UpstreamClient + ?Sized,
    C: CacheStore + ?Sized,
{
    let edition_pda = derive_edition_pda(mint_b58);
    if edition_pda.is_empty() {
        return;
    }
    let Ok(Some(account)) = upstream.get_account(&edition_pda).await else {
        return;
    };
    if account.data.is_empty() {
        return;
    }
    // Dispatch on Key discriminator. MasterEditionV2 is current; V1 is
    // deprecated but still live on-chain for older collections.
    match account.data[0] {
        k if k == Key::MasterEditionV2 as u8 || k == Key::MasterEditionV1 as u8 => {
            if let Ok(master) = MasterEdition::from_bytes(&account.data) {
                let _ = cache
                    .put_master_edition(MasterEditionRecord {
                        master_mint: mint_b58.to_string(),
                        master_edition_pda: edition_pda,
                        supply: master.supply,
                        max_supply: master.max_supply,
                    })
                    .await;
            }
        }
        k if k == Key::EditionV1 as u8 => {
            if let Ok(edition) = Edition::from_bytes(&account.data) {
                let _ = cache
                    .put_print_edition(PrintEditionRecord {
                        print_mint: mint_b58.to_string(),
                        print_edition_pda: edition_pda,
                        parent_master_edition_pda: edition.parent.to_string(),
                        edition_num: edition.edition,
                    })
                    .await;
            }
        }
        _ => {}
    }
}

fn derive_edition_pda(mint_b58: &str) -> String {
    let Ok(mint) = Pubkey::from_str(mint_b58) else {
        return String::new();
    };
    let Ok(program) = Pubkey::from_str(MPL_TOKEN_METADATA_PROGRAM) else {
        return String::new();
    };
    let (pda, _bump) = Pubkey::find_program_address(
        &[b"metadata", program.as_ref(), mint.as_ref(), b"edition"],
        &program,
    );
    pda.to_string()
}

/// Resolve the wallet that holds an NFT. `metadata_account` is the
/// Metaplex Metadata PDA account — we parse `mint` out of it and defer
/// to `resolve_owner_for_mint`. Kept as a thin wrapper so the core
/// two-RPC dance is directly testable without synthesizing Metadata
/// bytes (mpl-token-metadata 5.x uses a different solana-program major
/// than the local one, making in-test Metadata construction painful).
async fn resolve_token_metadata_owner<U>(
    upstream: &U,
    metadata_account: &AccountData,
) -> Option<String>
where
    U: UpstreamClient + ?Sized,
{
    let metadata = Metadata::from_bytes(&metadata_account.data).ok()?;
    resolve_owner_for_mint(upstream, &metadata.mint.to_string()).await
}

/// Core owner-resolution flow:
///   1. `getTokenLargestAccounts(mint)` → find a token account holding
///      the mint. For NFTs (supply=1) there's only ever one holder; for
///      zero-balance remnants (e.g. a closed ATA left behind after
///      burn) we skip them and take the next entry.
///   2. `getAccountInfo` on that token account; bytes 32..64 are the
///      owner wallet (SPL Token + Token-2022 share this offset for
///      the base account layout).
///
/// Returns `None` on any step failure — callers fall back to an empty
/// owner rather than 500ing the DAS response. Failure modes seen in
/// practice: a mint with zero holders (burned or pre-mint), an
/// upstream that doesn't implement `getTokenLargestAccounts`, or a
/// token account whose layout isn't recognized.
pub async fn resolve_owner_for_mint<U>(upstream: &U, mint: &str) -> Option<String>
where
    U: UpstreamClient + ?Sized,
{
    let raw = upstream
        .rpc_call("getTokenLargestAccounts", serde_json::json!([mint]))
        .await
        .ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let token_account_addr = parsed
        .get("value")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| {
            arr.iter()
                .find(|entry| entry.get("amount").and_then(serde_json::Value::as_str) != Some("0"))
        })
        .and_then(|entry| entry.get("address"))
        .and_then(serde_json::Value::as_str)?;

    let token_account = upstream.get_account(token_account_addr).await.ok()??;
    if token_account.data.len() < 64 {
        return None;
    }
    let mut owner_bytes = [0u8; 32];
    owner_bytes.copy_from_slice(&token_account.data[32..64]);
    Some(bs58::encode(owner_bytes).into_string())
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
    let (pda, _bump) =
        Pubkey::find_program_address(&[b"metadata", program.as_ref(), mint.as_ref()], &program);
    pda.to_string()
}
