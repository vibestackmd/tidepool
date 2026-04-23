//! Service-layer DAS handler functions. Async, `CnftStore`-backed.
//! Return domain types (`DasAsset`, `DasAssetProof`); wiring to
//! JSON-RPC lives in the server crate.
//!
//! Step 3a scope: cNFT-only paths. Uncompressed assets (MplCore,
//! Token Metadata) land in 3b when their decoders come online; until
//! then `get_asset` returns `Ok(None)` for anything not in the cNFT
//! index.

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;
use tidepool_core::{compute_proof, ProofError, TreeState};

use crate::cache::{CacheError, CacheStore, SearchFilter};
use crate::cnft::{CnftStore, LeafRecord};
use crate::upstream::UpstreamClient;

use super::cnft_to_das::leaf_record_to_das_asset;
use super::decoder::AccountDecoder;
use super::fetch::{fetch_and_cache_asset, FetchError};
use super::types::{
    DasAsset, DasAssetProof, DasBalances, DasNftEditionEntry, DasNftEditions, DasTokenAccount,
    DasTokenAccounts, DasTokenBalance,
};

#[derive(Debug, Error)]
pub enum DasError {
    #[error(transparent)]
    Store(#[from] crate::cnft::store::StoreError),
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("proof computation failed: {0}")]
    Proof(ProofError),
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("bad request: {0}")]
    BadRequest(String),
}

// ProofError doesn't derive Error, so we manually bridge.
impl From<ProofError> for DasError {
    fn from(e: ProofError) -> Self {
        Self::Proof(e)
    }
}

pub type DasResult<T> = Result<T, DasError>;

// ─── getAsset ──────────────────────────────────────────────────────

/// Compressed-only `getAsset`. Kept as a dedicated helper so tests
/// against a `CnftStore`-only ctx don't need upstream/cache wiring.
pub async fn get_asset<S: CnftStore + ?Sized>(
    cnft: &S,
    asset_id: &[u8; 32],
) -> DasResult<Option<DasAsset>> {
    if let Some(record) = cnft.get_leaf(asset_id).await? {
        return Ok(Some(leaf_record_to_das_asset(&record)));
    }
    Ok(None)
}

/// Full `getAsset` dispatch: cNFT store first, then cache, then
/// upstream fetch + decoder pipeline. Populates the cache on decode
/// success. `asset_id_b58` is the base58 address (mint or PDA) —
/// callers typically take this from JSON-RPC params untouched.
pub async fn get_asset_full<S, C, U>(
    cnft: &S,
    cache: &C,
    upstream: &U,
    decoders: &[Arc<dyn AccountDecoder>],
    asset_id_b58: &str,
) -> DasResult<Option<DasAsset>>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
{
    // 1. cNFT first. Decode base58 to [u8; 32]; unparseable ids fall
    //    through to the uncompressed path which may still resolve.
    if let Some(id_bytes) = try_decode_bs58_32(asset_id_b58) {
        if let Some(record) = cnft.get_leaf(&id_bytes).await? {
            return Ok(Some(leaf_record_to_das_asset(&record)));
        }
    }

    // 2. Cache hit — serves repeats without an upstream round-trip.
    if let Some(cached) = cache.get_asset(asset_id_b58).await? {
        return Ok(Some(cached));
    }

    // 3. Upstream fetch + decoder dispatch + cache populate.
    Ok(fetch_and_cache_asset(upstream, cache, decoders, asset_id_b58).await?)
}

// ─── getAssetBatch ─────────────────────────────────────────────────

/// Parallel `getAsset` over many ids. Preserves input order; unknown
/// ids map to `None`. Cache lookups run serially (fast, in-memory);
/// upstream fetches run in parallel via `futures` joins.
pub async fn get_asset_batch<S, C, U>(
    cnft: &S,
    cache: &C,
    upstream: &U,
    decoders: &[Arc<dyn AccountDecoder>],
    asset_ids_b58: &[String],
) -> DasResult<Vec<Option<DasAsset>>>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
{
    let mut out = Vec::with_capacity(asset_ids_b58.len());
    for id in asset_ids_b58 {
        // One-at-a-time keeps the lock-contention story simple on
        // MemoryCache's single Mutex. For larger batch fanouts we'd
        // spawn tasks; at current scale (batches up to 1000) the
        // upstream's own concurrency model is the bottleneck.
        out.push(get_asset_full(cnft, cache, upstream, decoders, id).await?);
    }
    Ok(out)
}

// ─── by-X handlers (cache-only) ────────────────────────────────────

pub async fn get_assets_by_owner<C: CacheStore + ?Sized>(
    cache: &C,
    owner: &str,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.get_assets_by_owner(owner).await?)
}

pub async fn get_assets_by_authority<C: CacheStore + ?Sized>(
    cache: &C,
    authority: &str,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.get_assets_by_authority(authority).await?)
}

pub async fn get_assets_by_creator<C: CacheStore + ?Sized>(
    cache: &C,
    creator: &str,
    only_verified: bool,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.get_assets_by_creator(creator, only_verified).await?)
}

pub async fn get_assets_by_group<C: CacheStore + ?Sized>(
    cache: &C,
    group_key: &str,
    group_value: &str,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.get_assets_by_group(group_key, group_value).await?)
}

pub async fn search_assets<C: CacheStore + ?Sized>(
    cache: &C,
    filter: &SearchFilter,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.search_assets(filter).await?)
}

fn try_decode_bs58_32(s: &str) -> Option<[u8; 32]> {
    let bytes = bs58::decode(s).into_vec().ok()?;
    bytes.try_into().ok()
}

// ─── getAssetProof ─────────────────────────────────────────────────

/// `getAssetProof` for a compressed asset. Materializes a `TreeState`
/// from all non-burned leaves in the tree, then runs the pure
/// `compute_proof`. Returns `Ok(None)` when the asset (or its tree)
/// isn't indexed.
pub async fn get_asset_proof<S: CnftStore + ?Sized>(
    cnft: &S,
    asset_id: &[u8; 32],
) -> DasResult<Option<DasAssetProof>> {
    let Some(leaf) = cnft.get_leaf(asset_id).await? else {
        return Ok(None);
    };
    let Some(tree_info) = cnft.get_tree(&leaf.tree).await? else {
        return Ok(None);
    };

    let state = build_tree_state(cnft, &leaf.tree, tree_info.depth).await?;
    let proof = compute_proof(&state, leaf.leaf_index)?;

    Ok(Some(DasAssetProof {
        root: bs58::encode(proof.root).into_string(),
        proof: proof
            .proof
            .iter()
            .map(|n| bs58::encode(n).into_string())
            .collect(),
        node_index: proof.node_index,
        leaf: bs58::encode(proof.leaf).into_string(),
        tree_id: bs58::encode(leaf.tree).into_string(),
        // We don't track an indexing slot locally — our Bubblegum
        // replay is cursor-driven, not slot-driven. Emit 0 so the
        // key is always present.
        last_indexed_slot: 0,
    }))
}

/// Parallel version. Preserves input order; unknown ids map to
/// `None`. Internally shares one `TreeState` materialization per
/// unique tree to keep N×proof requests from triggering N×
/// `list_leaves` scans.
pub async fn get_asset_proof_batch<S: CnftStore + ?Sized>(
    cnft: &S,
    asset_ids: &[[u8; 32]],
) -> DasResult<Vec<Option<DasAssetProof>>> {
    let mut results: Vec<Option<DasAssetProof>> = Vec::with_capacity(asset_ids.len());

    // First pass: look up every leaf + group by tree. This lets us
    // materialize each tree's state exactly once even for large
    // batches spanning multiple trees.
    let mut leaves: Vec<Option<LeafRecord>> = Vec::with_capacity(asset_ids.len());
    let mut trees_needed: BTreeMap<[u8; 32], u8> = BTreeMap::new();
    for id in asset_ids {
        let leaf = cnft.get_leaf(id).await?;
        if let Some(ref l) = leaf {
            if let std::collections::btree_map::Entry::Vacant(e) = trees_needed.entry(l.tree) {
                // Depth is the same for every leaf in a tree; we'll
                // fill it from TreeInfo below.
                e.insert(0);
            }
        }
        leaves.push(leaf);
    }

    // Pull each distinct tree's info + state once.
    let mut tree_states: BTreeMap<[u8; 32], TreeState> = BTreeMap::new();
    for tree in trees_needed.keys().copied().collect::<Vec<_>>() {
        let Some(info) = cnft.get_tree(&tree).await? else {
            continue;
        };
        let state = build_tree_state(cnft, &tree, info.depth).await?;
        tree_states.insert(tree, state);
    }

    for (i, id) in asset_ids.iter().enumerate() {
        let Some(leaf) = leaves[i].as_ref() else {
            results.push(None);
            continue;
        };
        let Some(state) = tree_states.get(&leaf.tree) else {
            results.push(None);
            continue;
        };
        let proof = compute_proof(state, leaf.leaf_index)?;
        results.push(Some(DasAssetProof {
            root: bs58::encode(proof.root).into_string(),
            proof: proof
                .proof
                .iter()
                .map(|n| bs58::encode(n).into_string())
                .collect(),
            node_index: proof.node_index,
            leaf: bs58::encode(proof.leaf).into_string(),
            tree_id: bs58::encode(leaf.tree).into_string(),
            last_indexed_slot: 0,
        }));
        // Discard the unused value from asset_ids iteration — kept
        // for index alignment with `results` + `leaves`.
        let _ = id;
    }

    Ok(results)
}

// ─── getNftEditions ────────────────────────────────────────────────

/// Serve `getNftEditions(mint, page, limit)`.
///
/// Pulls the master record + print editions from the cache's
/// LOCAL_INDEX populated as a side effect of `fetch_and_cache_asset`.
/// If we haven't fetched the master's `getAsset` yet, the upstream
/// fetch + edition-PDA indexing runs first so a cold-path call still
/// produces the master summary — the print list remains limited to
/// assets we've happened to see.
///
/// Pagination is 1-indexed to match Helius. Clamps out-of-range pages
/// to empty rather than erroring.
pub async fn get_nft_editions<C, U>(
    cache: &C,
    upstream: &U,
    decoders: &[Arc<dyn AccountDecoder>],
    master_mint: &str,
    page: u64,
    limit: u64,
) -> DasResult<Option<DasNftEditions>>
where
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
{
    // Warm the master's index by fetching its asset once; this is a
    // no-op if we've already fetched it.
    if cache.get_master_edition(master_mint).await?.is_none() {
        // We don't need a CnftStore here — `fetch_and_cache_asset` will
        // produce the master DasAsset + trigger the edition-PDA side
        // effect. Ignore errors; we still try to serve from whatever
        // the cache has.
        let _ = fetch_and_cache_asset(upstream, cache, decoders, master_mint).await;
    }

    let Some(master) = cache.get_master_edition(master_mint).await? else {
        return Ok(None);
    };

    let all_prints = cache
        .list_print_editions(&master.master_edition_pda)
        .await?;
    let total = all_prints.len() as u64;

    // 1-indexed pagination. A page of 0 or past the end just gives an
    // empty slice — mirrors how Helius handles overshoot.
    let limit = limit.max(1);
    let start = page.saturating_sub(1).saturating_mul(limit);
    let end = start.saturating_add(limit).min(total);
    let editions: Vec<DasNftEditionEntry> = if start >= total {
        Vec::new()
    } else {
        all_prints[start as usize..end as usize]
            .iter()
            .map(|r| DasNftEditionEntry {
                mint: r.print_mint.clone(),
                edition_address: r.print_edition_pda.clone(),
                edition: r.edition_num,
            })
            .collect()
    };

    Ok(Some(DasNftEditions {
        total,
        limit,
        page,
        master_edition_address: master.master_edition_pda.clone(),
        supply: master.supply,
        max_supply: master.max_supply,
        editions,
    }))
}

// ─── getTokenAccounts ──────────────────────────────────────────────

/// SPL Token program IDs. We query both when the caller gives us a
/// `programId` hint that's missing (common case); callers who only
/// care about one can pass it explicitly.
pub const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// Filter + pagination inputs for `get_token_accounts`. Mirrors
/// Helius's wire shape — either `owner` or `mint` must be provided;
/// supplying both ANDs them.
#[derive(Debug, Clone, Default)]
pub struct TokenAccountsFilter {
    pub owner: Option<String>,
    pub mint: Option<String>,
    pub page: u64,
    pub limit: u64,
    /// When false (default), zero-balance accounts are hidden —
    /// matches Helius's `displayOptions.showZeroBalance=false` default.
    pub show_zero_balance: bool,
}

/// Serve `getTokenAccounts`.
///
/// Implementation: when `owner` is set, forward to
/// `getTokenAccountsByOwner` (index-backed, fast). Otherwise fall back
/// to `getProgramAccounts` with a memcmp on the mint (slower, but only
/// runs when callers explicitly filter by mint without owner).
///
/// Always requests `jsonParsed` encoding so we can read amounts + flags
/// without hand-decoding SPL Token's 165-byte layout.
pub async fn get_token_accounts<U: UpstreamClient + ?Sized>(
    upstream: &U,
    filter: &TokenAccountsFilter,
) -> DasResult<DasTokenAccounts> {
    let limit = filter.limit.max(1);
    let page = filter.page.max(1);

    // Validate at least one filter. Mirroring Helius, which rejects
    // the no-filter form instead of dumping the entire token universe.
    if filter.owner.is_none() && filter.mint.is_none() {
        return Err(DasError::BadRequest(
            "getTokenAccounts requires at least one of `owner` or `mint`".into(),
        ));
    }

    // Query both SPL Token and Token-2022 so tokens minted under the
    // newer program aren't silently dropped.
    let mut accounts: Vec<DasTokenAccount> = Vec::new();
    for program_id in [SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID] {
        let entries = if let Some(owner) = &filter.owner {
            fetch_by_owner(upstream, owner, filter.mint.as_deref(), program_id).await?
        } else {
            // `mint` is Some here by the validation above.
            fetch_by_mint(upstream, filter.mint.as_deref().unwrap(), program_id).await?
        };
        accounts.extend(entries);
    }

    if !filter.show_zero_balance {
        accounts.retain(|a| a.amount > 0);
    }

    // Deterministic order for pagination stability — by address.
    accounts.sort_by(|a, b| a.address.cmp(&b.address));

    let total = accounts.len() as u64;
    let start = page.saturating_sub(1).saturating_mul(limit);
    let end = start.saturating_add(limit).min(total);
    let page_accounts: Vec<DasTokenAccount> = if start >= total {
        Vec::new()
    } else {
        accounts[start as usize..end as usize].to_vec()
    };

    Ok(DasTokenAccounts {
        total,
        limit,
        page,
        token_accounts: page_accounts,
    })
}

async fn fetch_by_owner<U: UpstreamClient + ?Sized>(
    upstream: &U,
    owner: &str,
    mint: Option<&str>,
    program_id: &str,
) -> DasResult<Vec<DasTokenAccount>> {
    // `getTokenAccountsByOwner` takes either `mint` or `programId` as
    // the filter. Passing `mint` narrows further; without it we pass
    // `programId` to scope the query to SPL Token vs Token-2022.
    let filter = if let Some(m) = mint {
        serde_json::json!({ "mint": m })
    } else {
        serde_json::json!({ "programId": program_id })
    };
    let params = serde_json::json!([
        owner,
        filter,
        { "encoding": "jsonParsed", "commitment": "confirmed" }
    ]);
    let raw = upstream
        .rpc_call("getTokenAccountsByOwner", params)
        .await
        .map_err(|e| DasError::Upstream(e.to_string()))?;
    Ok(parse_token_account_list(&raw))
}

async fn fetch_by_mint<U: UpstreamClient + ?Sized>(
    upstream: &U,
    mint: &str,
    program_id: &str,
) -> DasResult<Vec<DasTokenAccount>> {
    // memcmp on offset 0 of the SPL Token Account layout (mint field).
    let params = serde_json::json!([
        program_id,
        {
            "encoding": "jsonParsed",
            "commitment": "confirmed",
            "filters": [
                { "dataSize": 165 },
                { "memcmp": { "offset": 0, "bytes": mint } }
            ]
        }
    ]);
    let raw = upstream
        .rpc_call("getProgramAccounts", params)
        .await
        .map_err(|e| DasError::Upstream(e.to_string()))?;
    // `getProgramAccounts` without `withContext: true` returns a bare
    // array; the by-owner shape wraps it in `{context, value}`. Handle
    // both shapes uniformly.
    Ok(parse_token_account_list(&raw))
}

/// Parse either a bare `[{pubkey, account}, ...]` array or the
/// `{context, value: [...] }` envelope into our shape. Silently skips
/// entries that don't look like parsed SPL Token accounts — malformed
/// entries never surface as errors, we just drop them.
fn parse_token_account_list(raw: &[u8]) -> Vec<DasTokenAccount> {
    use serde_json::Value;

    let value: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let arr = if let Some(inner) = value.get("value") {
        inner.as_array().cloned().unwrap_or_default()
    } else {
        value.as_array().cloned().unwrap_or_default()
    };

    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let Some(address) = entry.get("pubkey").and_then(Value::as_str) else {
            continue;
        };
        let info = entry
            .pointer("/account/data/parsed/info")
            .cloned()
            .unwrap_or(Value::Null);
        let Some(mint) = info.get("mint").and_then(Value::as_str) else {
            continue;
        };
        let Some(owner) = info.get("owner").and_then(Value::as_str) else {
            continue;
        };
        // `amount` is a decimal string in jsonParsed output (matches
        // Solana RPC's stringified-u64 convention for amounts > 2^53).
        let amount = info
            .get("tokenAmount")
            .and_then(|t| t.get("amount"))
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let delegate = info
            .get("delegate")
            .and_then(Value::as_str)
            .map(String::from);
        let delegated_amount = info
            .get("delegatedAmount")
            .and_then(|t| t.get("amount"))
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        // SPL's `state` field is either "initialized", "frozen", or
        // "uninitialized" in jsonParsed output. Only "frozen" matters
        // for our frozen flag.
        let frozen = info
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(|s| s == "frozen");
        out.push(DasTokenAccount {
            address: address.to_string(),
            mint: mint.to_string(),
            owner: owner.to_string(),
            amount,
            delegated_amount,
            frozen,
            delegate,
        });
    }
    out
}

// ─── getBalances (Wallet API) ──────────────────────────────────────

/// Serve `getBalances(owner)` — native SOL position + every SPL Token
/// / Token-2022 position held by the wallet.
///
/// Three upstream calls: `getBalance` for lamports, then
/// `getTokenAccountsByOwner` against each of SPL Token and Token-2022.
/// USD pricing (`priceInUSD`, `totalPrice`) is left null: we don't have
/// a price feed locally and don't plan to add one (scope deliberately
/// excluded in the coverage doc).
pub async fn get_balances<U: UpstreamClient + ?Sized>(
    upstream: &U,
    owner: &str,
) -> DasResult<DasBalances> {
    let lamports = fetch_native_balance(upstream, owner).await?;

    let mut tokens: Vec<DasTokenBalance> = Vec::new();
    for program_id in [SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID] {
        tokens.extend(fetch_token_balances(upstream, owner, program_id).await?);
    }
    // Deterministic order for stability — mint then account.
    tokens.sort_by(|a, b| {
        a.mint
            .cmp(&b.mint)
            .then_with(|| a.token_account.cmp(&b.token_account))
    });

    Ok(DasBalances {
        native_balance: lamports,
        tokens,
    })
}

async fn fetch_native_balance<U: UpstreamClient + ?Sized>(
    upstream: &U,
    owner: &str,
) -> DasResult<u64> {
    let raw = upstream
        .rpc_call("getBalance", serde_json::json!([owner]))
        .await
        .map_err(|e| DasError::Upstream(e.to_string()))?;
    let parsed: serde_json::Value = serde_json::from_slice(&raw).unwrap_or(serde_json::Value::Null);
    // `getBalance` returns `{context, value}`; value is the u64
    // lamports directly.
    let lamports = parsed
        .get("value")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok(lamports)
}

async fn fetch_token_balances<U: UpstreamClient + ?Sized>(
    upstream: &U,
    owner: &str,
    program_id: &str,
) -> DasResult<Vec<DasTokenBalance>> {
    let params = serde_json::json!([
        owner,
        { "programId": program_id },
        { "encoding": "jsonParsed", "commitment": "confirmed" }
    ]);
    let raw = upstream
        .rpc_call("getTokenAccountsByOwner", params)
        .await
        .map_err(|e| DasError::Upstream(e.to_string()))?;
    Ok(parse_token_balance_list(&raw))
}

fn parse_token_balance_list(raw: &[u8]) -> Vec<DasTokenBalance> {
    use serde_json::Value;

    let value: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let arr = value
        .get("value")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let Some(token_account) = entry.get("pubkey").and_then(Value::as_str) else {
            continue;
        };
        let info = entry
            .pointer("/account/data/parsed/info")
            .cloned()
            .unwrap_or(Value::Null);
        let Some(mint) = info.get("mint").and_then(Value::as_str) else {
            continue;
        };
        let token_amount = info.get("tokenAmount").cloned().unwrap_or(Value::Null);
        let amount = token_amount
            .get("amount")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        // Zero-balance ATAs don't belong in a balances view — hide them.
        if amount == 0 {
            continue;
        }
        let decimals = token_amount
            .get("decimals")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u8;
        out.push(DasTokenBalance {
            token_account: token_account.to_string(),
            mint: mint.to_string(),
            amount,
            decimals,
            price_in_usd: None,
            total_price: None,
        });
    }
    out
}

// ─── helpers ───────────────────────────────────────────────────────

async fn build_tree_state<S: CnftStore + ?Sized>(
    cnft: &S,
    tree: &[u8; 32],
    depth: u8,
) -> DasResult<TreeState> {
    let mut leaves = BTreeMap::new();
    for rec in cnft.list_leaves(tree).await? {
        if !rec.burned {
            leaves.insert(rec.leaf_index, rec.leaf_hash);
        }
    }
    Ok(TreeState { depth, leaves })
}
