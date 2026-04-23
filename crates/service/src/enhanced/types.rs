//! Wire shapes for enhanced transactions. Field names mirror Helius's
//! camelCase JSON.

use serde::{Deserialize, Serialize};

/// One native-SOL transfer extracted from the pre/postBalances diff.
/// `amount` is lamports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedNativeTransfer {
    pub from_user_account: String,
    pub to_user_account: String,
    pub amount: u64,
}

/// One SPL-Token transfer. `token_amount` is raw u64 (we don't divide
/// by decimals — matches Helius's default behavior of returning the
/// integer amount for precision). `mint` is the SPL mint pubkey.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedTokenTransfer {
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    pub from_token_account: Option<String>,
    pub to_token_account: Option<String>,
    pub mint: String,
    pub token_amount: u64,
}

/// Skeleton of an instruction in the enhanced envelope. We preserve
/// the outer ix shape + inner ixs verbatim; the real Helius shape
/// includes decoded per-program fields under `parsed` which our
/// classifier doesn't populate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedInstruction {
    pub program_id: String,
    pub accounts: Vec<String>,
    pub data: String,
    #[serde(default)]
    pub inner_instructions: Vec<EnhancedInstruction>,
}

/// One fully-enhanced transaction. `tx_type` + `source` drive the
/// client's per-type rendering; everything else is raw data Helius
/// surfaces at the top level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedTransaction {
    pub signature: String,
    pub slot: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    #[serde(rename = "type")]
    pub tx_type: String,
    pub source: String,
    pub fee: u64,
    pub fee_payer: String,
    pub description: String,
    pub native_transfers: Vec<EnhancedNativeTransfer>,
    pub token_transfers: Vec<EnhancedTokenTransfer>,
    pub instructions: Vec<EnhancedInstruction>,
    /// Per-type event breakouts. Currently populated only for NFT-
    /// flavored transactions (mints, sales, transfers, burns); empty
    /// object otherwise. Helius also surfaces `compressed`, `swap`,
    /// `stake` sub-fields — we don't populate those yet.
    pub events: EnhancedEvents,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_error: Option<serde_json::Value>,
}

/// Per-type event breakouts. Every field is optional — serializes as
/// `{}` when nothing is populated, so the key is always present but
/// callers can skip deserialization of empty sub-objects cheaply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnhancedEvents {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nft: Option<NftEvent>,
}

/// Heuristic NFT event derived from the classifier + transfers.
/// Deliberately minimal — type, optional mint, and the derived
/// buyer/seller/amount when we can pull them out of native/token
/// transfer diffs. Helius's production shape has more fields
/// (staker, signature list, saleType, etc.) we deliberately don't
/// populate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NftEvent {
    /// Mirrors the parent `tx_type` for the NFT family (NFT_MINT,
    /// NFT_TRANSFER, NFT_BURN, COMPRESSED_NFT_MINT, etc.) so clients
    /// reading `events.nft` in isolation get the same classification.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Same `source` tag the top-level envelope carries.
    pub source: String,
    /// Best-effort mint identifier when one transfer line is
    /// obviously the NFT move. Null when we can't pin it down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nfts: Option<Vec<NftEventMint>>,
    /// Derived sale amount in lamports — single biggest native
    /// transfer, if any. Useful for NFT_SALE inference even though
    /// the classifier doesn't itself emit NFT_SALE yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buyer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seller: Option<String>,
}

/// One NFT identifier within a `NftEvent`. A single tx can mint or
/// move several NFTs; we record each.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftEventMint {
    pub mint: String,
    pub token_standard: String,
}
