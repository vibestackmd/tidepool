//! Event → store mutation. One place where CnftEvent variants turn
//! into `CnftStore` writes. Keeping this separate from the parser and
//! the indexer lets us unit-test state transitions against a pure
//! event stream, independent of ix decoding or upstream RPC.
//!
//! Authoritative-state policy: when `event.noop` is present (optional
//! on mint/transfer/burn/delegate, required on the verify family and
//! updateMetadata), we use its values directly. On the optional-noop
//! variants we fall back to reconstructing from ix args + stored
//! state. The type system makes this impossible to forget — noop-
//! required variants won't compile without it.

use mpl_bubblegum::ID as BUBBLEGUM_PROGRAM_PUBKEY;
use solana_program::pubkey::Pubkey;
use thiserror::Error;
use tidepool_rpc_core::{hash_creators, hash_leaf_v1, hash_metadata_args_bytes, LeafSchemaV1};

use super::store::{CnftStore, StoreError};
use super::types::{CnftEvent, LeafRecord, MintMetadata, NoopOverride, TreeInfo};

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("mint on unknown tree")]
    UnknownTree,
    #[error("expected leaf at (tree, {leaf_index}) but none exists")]
    MissingLeaf { leaf_index: u64 },
}

pub type ApplyResult<T> = Result<T, ApplyError>;

/// Apply one parsed event to the store. Idempotent in the sense that
/// re-applying an event produces the same end state — but **not**
/// commutative: order matters, callers must apply in signature-
/// chronological order.
#[allow(clippy::too_many_lines)]
pub async fn apply_event<S: CnftStore + ?Sized>(store: &S, event: CnftEvent) -> ApplyResult<()> {
    match event {
        CnftEvent::CreateTree {
            tree,
            depth,
            max_buffer_size,
        } => {
            store
                .put_tree(TreeInfo {
                    tree,
                    depth,
                    max_buffer_size,
                    num_minted: 0,
                })
                .await?;
        }

        CnftEvent::Mint {
            tree,
            owner,
            delegate,
            metadata,
            verify_collection: _,
            noop,
        } => {
            apply_mint(store, tree, owner, delegate, metadata, noop.as_ref()).await?;
        }

        CnftEvent::Transfer {
            tree,
            leaf_index,
            nonce: _,
            new_owner,
            new_delegate,
            data_hash,
            creator_hash,
            noop,
        } => {
            let existing = require_leaf(store, &tree, leaf_index).await?;

            // Optional noop override wins whenever present. Otherwise
            // reconstruct from ix args + existing state. We also
            // sanity-check that the caller-asserted pre-image hashes
            // match what we have — divergence means stale local
            // state, and we skip silently so a future re-scan can
            // catch up.
            if noop.is_none()
                && (existing.data_hash != data_hash || existing.creator_hash != creator_hash)
            {
                return Ok(());
            }

            let (owner, delegate, data_hash, creator_hash, leaf_hash_override) = match noop.as_ref()
            {
                Some(ov) => (
                    ov.owner,
                    ov.delegate,
                    ov.data_hash,
                    ov.creator_hash,
                    Some(ov.leaf_hash),
                ),
                None => (
                    new_owner,
                    new_delegate,
                    existing.data_hash,
                    existing.creator_hash,
                    None,
                ),
            };
            put_updated_leaf(
                store,
                existing,
                owner,
                delegate,
                data_hash,
                creator_hash,
                None,
                leaf_hash_override,
            )
            .await?;
        }

        CnftEvent::Delegate {
            tree,
            leaf_index,
            nonce: _,
            new_delegate,
            data_hash,
            creator_hash,
            noop,
        } => {
            let existing = require_leaf(store, &tree, leaf_index).await?;
            if noop.is_none()
                && (existing.data_hash != data_hash || existing.creator_hash != creator_hash)
            {
                return Ok(());
            }
            let (owner, delegate, data_hash, creator_hash, leaf_hash_override) = match noop.as_ref()
            {
                Some(ov) => (
                    ov.owner,
                    ov.delegate,
                    ov.data_hash,
                    ov.creator_hash,
                    Some(ov.leaf_hash),
                ),
                None => (
                    existing.owner,
                    new_delegate,
                    existing.data_hash,
                    existing.creator_hash,
                    None,
                ),
            };
            put_updated_leaf(
                store,
                existing,
                owner,
                delegate,
                data_hash,
                creator_hash,
                None,
                leaf_hash_override,
            )
            .await?;
        }

        CnftEvent::Burn {
            tree,
            leaf_index,
            nonce: _,
            noop: _,
        } => {
            let existing = require_leaf(store, &tree, leaf_index).await?;
            let next = LeafRecord {
                burned: true,
                leaf_hash: [0u8; 32],
                ..existing
            };
            store.put_leaf(next).await?;
        }

        CnftEvent::VerifyCreator {
            tree,
            creator,
            noop,
        } => {
            apply_creator_flip(store, &tree, &creator, true, &noop).await?;
        }
        CnftEvent::UnverifyCreator {
            tree,
            creator,
            noop,
        } => {
            apply_creator_flip(store, &tree, &creator, false, &noop).await?;
        }

        // VerifyCollection and SetAndVerifyCollection both mark the
        // collection as verified; the only difference is that
        // SetAndVerifyCollection can also overwrite the collection
        // key, which apply_collection_flip does unconditionally.
        CnftEvent::VerifyCollection {
            tree,
            collection,
            noop,
        }
        | CnftEvent::SetAndVerifyCollection {
            tree,
            collection,
            noop,
        } => {
            apply_collection_flip(store, &tree, &collection, true, &noop).await?;
        }
        CnftEvent::UnverifyCollection {
            tree,
            collection,
            noop,
        } => {
            apply_collection_flip(store, &tree, &collection, false, &noop).await?;
        }

        CnftEvent::UpdateMetadata {
            tree,
            new_metadata,
            noop,
        } => {
            apply_update_metadata(store, &tree, new_metadata, &noop).await?;
        }
    }
    Ok(())
}

// ─── mint ──────────────────────────────────────────────────────────

async fn apply_mint<S: CnftStore + ?Sized>(
    store: &S,
    tree: [u8; 32],
    owner: [u8; 32],
    delegate: [u8; 32],
    metadata: MintMetadata,
    noop: Option<&NoopOverride>,
) -> ApplyResult<()> {
    let tree_info = store
        .get_tree(&tree)
        .await?
        .ok_or(ApplyError::UnknownTree)?;

    // Nonce assignment: noop is authoritative if present; otherwise
    // we allocate the next leaf index from the tree's counter.
    let leaf_index = if let Some(ov) = noop {
        store
            .ensure_num_minted_at_least(&tree, ov.nonce + 1)
            .await?;
        ov.nonce
    } else {
        store.alloc_leaf_index(&tree).await?
    };
    let nonce = leaf_index;

    // Hashes: prefer noop-authoritative; otherwise reconstruct. For
    // asset_id, V2 mints emit their own id in the noop so we use that
    // directly (happens to match `derive_asset_id` for the current
    // Bubblegum derivation, but staying authoritative future-proofs us).
    let (asset_id, owner, delegate, data_hash, creator_hash, leaf_hash) = if let Some(ov) = noop {
        (
            ov.id,
            ov.owner,
            ov.delegate,
            ov.data_hash,
            ov.creator_hash,
            ov.leaf_hash,
        )
    } else {
        let asset_id = derive_asset_id(&tree, nonce);
        let data_hash = hash_metadata_args_bytes(&metadata.data_hash_input);
        let creator_hash = hash_creators(&metadata.creators);
        let leaf_hash = hash_leaf_v1(&LeafSchemaV1 {
            id: asset_id,
            owner,
            delegate,
            nonce,
            data_hash,
            creator_hash,
        });
        (
            asset_id,
            owner,
            delegate,
            data_hash,
            creator_hash,
            leaf_hash,
        )
    };

    store
        .put_leaf(LeafRecord {
            asset_id,
            tree,
            nonce,
            leaf_index,
            mint_metadata: metadata,
            owner,
            delegate,
            data_hash,
            creator_hash,
            leaf_hash,
            burned: false,
        })
        .await?;

    // Silence unused binding (tree_info is fetched to assert the tree
    // exists; the counter was already bumped via ensure_* / alloc_*).
    let _ = tree_info;
    Ok(())
}

// ─── creator / collection flips ────────────────────────────────────

async fn apply_creator_flip<S: CnftStore + ?Sized>(
    store: &S,
    tree: &[u8; 32],
    creator: &[u8; 32],
    verified: bool,
    noop: &NoopOverride,
) -> ApplyResult<()> {
    let existing = require_leaf(store, tree, noop.leaf_index).await?;
    let mut new_metadata = existing.mint_metadata.clone();
    for c in &mut new_metadata.creators {
        if &c.address == creator {
            c.verified = verified;
        }
    }
    put_updated_leaf(
        store,
        existing,
        noop.owner,
        noop.delegate,
        noop.data_hash,
        noop.creator_hash,
        Some(new_metadata),
        Some(noop.leaf_hash),
    )
    .await
}

async fn apply_collection_flip<S: CnftStore + ?Sized>(
    store: &S,
    tree: &[u8; 32],
    collection: &[u8; 32],
    verified: bool,
    noop: &NoopOverride,
) -> ApplyResult<()> {
    let existing = require_leaf(store, tree, noop.leaf_index).await?;
    let mut new_metadata = existing.mint_metadata.clone();
    new_metadata.collection = Some((*collection, verified));
    put_updated_leaf(
        store,
        existing,
        noop.owner,
        noop.delegate,
        noop.data_hash,
        noop.creator_hash,
        Some(new_metadata),
        Some(noop.leaf_hash),
    )
    .await
}

async fn apply_update_metadata<S: CnftStore + ?Sized>(
    store: &S,
    tree: &[u8; 32],
    new_metadata: MintMetadata,
    noop: &NoopOverride,
) -> ApplyResult<()> {
    let existing = require_leaf(store, tree, noop.leaf_index).await?;
    // Merge: any empty/default field in `new_metadata` keeps the
    // prior value (None in the wire args turned into default values
    // at the parser; here we treat defaults as "not provided").
    let prev = &existing.mint_metadata;
    let merged = MintMetadata {
        name: if new_metadata.name.is_empty() {
            prev.name.clone()
        } else {
            new_metadata.name
        },
        symbol: if new_metadata.symbol.is_empty() {
            prev.symbol.clone()
        } else {
            new_metadata.symbol
        },
        uri: if new_metadata.uri.is_empty() {
            prev.uri.clone()
        } else {
            new_metadata.uri
        },
        seller_fee_basis_points: if new_metadata.seller_fee_basis_points == 0 {
            prev.seller_fee_basis_points
        } else {
            new_metadata.seller_fee_basis_points
        },
        primary_sale_happened: new_metadata.primary_sale_happened || prev.primary_sale_happened,
        is_mutable: new_metadata.is_mutable,
        creators: if new_metadata.creators.is_empty() {
            prev.creators.clone()
        } else {
            new_metadata.creators
        },
        collection: prev.collection, // updateMetadata never changes collection membership
        data_hash_input: new_metadata.data_hash_input,
    };
    put_updated_leaf(
        store,
        existing,
        noop.owner,
        noop.delegate,
        noop.data_hash,
        noop.creator_hash,
        Some(merged),
        Some(noop.leaf_hash),
    )
    .await
}

// ─── shared plumbing ────────────────────────────────────────────────

async fn require_leaf<S: CnftStore + ?Sized>(
    store: &S,
    tree: &[u8; 32],
    leaf_index: u64,
) -> ApplyResult<LeafRecord> {
    store
        .get_leaf_by_index(tree, leaf_index)
        .await?
        .ok_or(ApplyError::MissingLeaf { leaf_index })
}

#[allow(clippy::too_many_arguments)]
async fn put_updated_leaf<S: CnftStore + ?Sized>(
    store: &S,
    existing: LeafRecord,
    owner: [u8; 32],
    delegate: [u8; 32],
    data_hash: [u8; 32],
    creator_hash: [u8; 32],
    new_metadata: Option<MintMetadata>,
    leaf_hash_override: Option<[u8; 32]>,
) -> ApplyResult<()> {
    // For V2 leaves (or whenever we have an authoritative noop emit),
    // the leaf_hash from the event folds in schema-specific fields
    // we don't track (collection_hash, asset_data_hash, flags). When
    // no override is given, fall back to V1 reconstruction.
    let leaf_hash = leaf_hash_override.unwrap_or_else(|| {
        hash_leaf_v1(&LeafSchemaV1 {
            id: existing.asset_id,
            owner,
            delegate,
            nonce: existing.nonce,
            data_hash,
            creator_hash,
        })
    });
    let next = LeafRecord {
        mint_metadata: new_metadata.unwrap_or_else(|| existing.mint_metadata.clone()),
        owner,
        delegate,
        data_hash,
        creator_hash,
        leaf_hash,
        ..existing
    };
    store.put_leaf(next).await?;
    Ok(())
}

/// Bubblegum asset ID derivation. Matches `mpl_bubblegum::get_asset_id`:
/// PDA of `["asset", tree, nonce.to_le_bytes()]` under the Bubblegum
/// program. We call out to the mpl-bubblegum helper so we track
/// upstream without hand-maintaining the derivation.
#[must_use]
pub fn derive_asset_id(tree: &[u8; 32], nonce: u64) -> [u8; 32] {
    let tree_pk = Pubkey::new_from_array(*tree);
    let (pda, _bump) = Pubkey::find_program_address(
        &[b"asset", tree_pk.as_ref(), &nonce.to_le_bytes()],
        &BUBBLEGUM_PROGRAM_PUBKEY,
    );
    pda.to_bytes()
}
