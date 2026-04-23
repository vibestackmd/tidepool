//! Pure extraction layer: take a `getTransaction` response JSON, walk
//! the outer + inner instructions, emit every Bubblegum ix paired
//! with its noop LeafSchemaEvent (when present).
//!
//! This is the one place that understands Solana's tx-wire shape. We
//! intentionally don't depend on `solana-transaction-status` here — it
//! pulls a large tree, and the handful of fields we need are trivial
//! to model via serde. Kept separate from the indexer orchestrator so
//! the mapping is easy to unit-test with canned fixtures — no RPC
//! mocking required.
//!
//! Rust-portability note: the shapes below are deliberately loose
//! (mostly `Option<Vec<...>>` with serde defaults) because Solana RPC
//! versions differ on optional fields. We read defensively and skip
//! anything we can't resolve.

use serde::Deserialize;

use super::leaf_event::{decode_leaf_schema_event, is_noop_program};
use super::parser::BUBBLEGUM_PROGRAM_ID;
use super::LeafSchemaEventDecoded;

/// One Bubblegum ix extracted from a tx, with its account list
/// resolved to 32-byte arrays and (optionally) the paired noop
/// LeafSchemaEvent pre-decoded.
#[derive(Debug, Clone)]
pub struct ExtractedIx {
    pub data: Vec<u8>,
    pub accounts: Vec<[u8; 32]>,
    pub noop_event: Option<LeafSchemaEventDecoded>,
}

// ─── wire-shape types ───────────────────────────────────────────────
// Deliberately narrow — we only name the fields this module reads.
// Serde's `default` + `Option<...>` handle missing / null gracefully.

#[derive(Debug, Deserialize, Default)]
pub struct RpcTransactionResponse {
    #[serde(default)]
    pub meta: Option<RpcTransactionMeta>,
    #[serde(default)]
    pub transaction: Option<RpcTransactionInner>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RpcTransactionMeta {
    /// `null` on success, an object on failure. Encoded generically —
    /// we only check for `null`.
    #[serde(default)]
    pub err: Option<serde_json::Value>,
    #[serde(default, rename = "innerInstructions")]
    pub inner_instructions: Option<Vec<InnerInstructionGroup>>,
    #[serde(default, rename = "loadedAddresses")]
    pub loaded_addresses: Option<LoadedAddresses>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LoadedAddresses {
    #[serde(default)]
    pub writable: Vec<String>,
    #[serde(default)]
    pub readonly: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct InnerInstructionGroup {
    pub index: u32,
    pub instructions: Vec<CompactInstruction>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CompactInstruction {
    #[serde(rename = "programIdIndex")]
    pub program_id_index: u32,
    #[serde(default)]
    pub accounts: Vec<u32>,
    pub data: String, // base58
}

#[derive(Debug, Deserialize)]
pub struct RpcTransactionInner {
    pub message: RpcMessage,
}

#[derive(Debug, Deserialize)]
pub struct RpcMessage {
    #[serde(rename = "accountKeys")]
    pub account_keys: Vec<String>,
    #[serde(default)]
    pub instructions: Vec<CompactInstruction>,
}

// ─── extraction ─────────────────────────────────────────────────────

/// Extract every Bubblegum ix (outer + inner) from a `getTransaction`
/// response, pairing each outer ix with the first LeafSchemaEvent
/// found under its inner-ix group. Ix order is preserved so state
/// transitions replay correctly.
///
/// - Txs that failed on-chain (`meta.err != null`) yield an empty vec.
/// - Inner Bubblegum ixs (Bubblegum called via CPI from a wrapper) are
///   paired with noop events appearing *after* them in the same inner
///   group — covers the Candy Guard / wrapper-program use case.
/// - Account indices that can't be resolved (bad base58, out-of-range)
///   cause the specific ix to be silently skipped, never the whole tx.
#[must_use]
pub fn extract_bubblegum_ixs(tx: &RpcTransactionResponse) -> Vec<ExtractedIx> {
    let Some(meta) = tx.meta.as_ref() else {
        return Vec::new();
    };
    // err is Some(null) on success under most RPC shapes; treat that
    // as not-an-error. Only a non-null Some is a failure.
    if meta.err.as_ref().is_some_and(|v| !v.is_null()) {
        return Vec::new();
    }
    let Some(tx_inner) = tx.transaction.as_ref() else {
        return Vec::new();
    };
    let message = &tx_inner.message;

    // Resolve the full keytable: static keys, then loaded-writable,
    // then loaded-readonly. Decode each base58 address into 32 bytes
    // once; positional lookups later index into this vec.
    let mut keys: Vec<Option<[u8; 32]>> = Vec::with_capacity(message.account_keys.len() + 32);
    for s in &message.account_keys {
        keys.push(decode_pubkey(s));
    }
    if let Some(la) = meta.loaded_addresses.as_ref() {
        for s in &la.writable {
            keys.push(decode_pubkey(s));
        }
        for s in &la.readonly {
            keys.push(decode_pubkey(s));
        }
    }
    // Mirror the string form too so noop-program checks stay cheap —
    // is_noop_program compares against the canonical base58 strings.
    let key_strings: Vec<&str> = {
        let mut v = Vec::with_capacity(keys.len());
        for s in &message.account_keys {
            v.push(s.as_str());
        }
        if let Some(la) = meta.loaded_addresses.as_ref() {
            for s in &la.writable {
                v.push(s.as_str());
            }
            for s in &la.readonly {
                v.push(s.as_str());
            }
        }
        v
    };

    let mut out = Vec::new();
    let empty_inner: Vec<InnerInstructionGroup> = Vec::new();
    let inner_ixs = meta.inner_instructions.as_ref().unwrap_or(&empty_inner);

    for (i, ix) in message.instructions.iter().enumerate() {
        let inner_group = inner_ixs
            .iter()
            .find(|g| g.index as usize == i)
            .map_or(&[][..], |g| g.instructions.as_slice());

        if is_bubblegum(&key_strings, ix.program_id_index as usize) {
            if let Some(mut extracted) = resolve_ix(&keys, ix) {
                extracted.noop_event = find_first_leaf_event(&key_strings, inner_group, 0);
                out.push(extracted);
            }
        }

        for (j, inner) in inner_group.iter().enumerate() {
            if is_bubblegum(&key_strings, inner.program_id_index as usize) {
                if let Some(mut extracted) = resolve_ix(&keys, inner) {
                    // Bubblegum CPI'd from a wrapper emits its own
                    // noop LeafSchemaEvent later in the inner-ix list.
                    extracted.noop_event = find_first_leaf_event(&key_strings, inner_group, j + 1);
                    out.push(extracted);
                }
            }
        }
    }

    out
}

fn is_bubblegum(keys: &[&str], program_id_index: usize) -> bool {
    keys.get(program_id_index).copied() == Some(BUBBLEGUM_PROGRAM_ID)
}

fn resolve_ix(keys: &[Option<[u8; 32]>], ix: &CompactInstruction) -> Option<ExtractedIx> {
    // Resolve every account index or bail — half-resolved accounts
    // would mean the parser reads garbage at the positions we care
    // about.
    let mut accounts = Vec::with_capacity(ix.accounts.len());
    for &idx in &ix.accounts {
        let key = keys.get(idx as usize).copied().flatten()?;
        accounts.push(key);
    }
    let data = bs58::decode(&ix.data).into_vec().ok()?;
    Some(ExtractedIx {
        data,
        accounts,
        noop_event: None,
    })
}

fn find_first_leaf_event(
    keys: &[&str],
    instructions: &[CompactInstruction],
    from_index: usize,
) -> Option<LeafSchemaEventDecoded> {
    for ix in instructions.iter().skip(from_index) {
        let program_key = keys.get(ix.program_id_index as usize).copied()?;
        if !is_noop_program(program_key) {
            continue;
        }
        let Ok(bytes) = bs58::decode(&ix.data).into_vec() else {
            continue;
        };
        if let Some(event) = decode_leaf_schema_event(&bytes) {
            return Some(event);
        }
    }
    None
}

/// Best-effort base58 → 32-byte pubkey. Returns None on any decode
/// failure; tx_extract treats unresolvable addresses as "skip this
/// ix" rather than "fail the whole tx."
fn decode_pubkey(s: &str) -> Option<[u8; 32]> {
    let bytes = bs58::decode(s).into_vec().ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Some(out)
}
