//! DAS response shapes — what Helius returns from `getAsset`,
//! `searchAssets`, and friends. Serialization is chosen per-field to
//! match Helius's wire format exactly (some fields are snake_case,
//! some are camelCase; serde rename attributes pin each one).
//!
//! Address + hash fields are `String` here (base58) because this is
//! the external wire contract. Internal store types use `[u8; 32]`;
//! conversion happens at the DAS-mapping boundary in `cnft_to_das`
//! and (later) the uncompressed decoders.

use serde::{Deserialize, Serialize};

/// DAS asset as returned by Helius. Optional fields are omitted from
/// serialization when absent so cNFT vs uncompressed shapes stay
/// self-documenting to clients.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasAsset {
    pub id: String,
    pub interface: String,
    pub content: DasContent,
    pub authorities: Vec<DasAuthority>,
    pub creators: Vec<DasCreator>,
    pub ownership: DasOwnership,
    pub grouping: Vec<DasGrouping>,
    pub mutable: bool,
    pub burnt: bool,

    /// Compression info — always emitted by Helius, as `null` for
    /// uncompressed assets and a populated object for cNFTs. We
    /// always serialize it (as null when None) to match.
    #[serde(default)]
    pub compression: Option<DasCompression>,
    /// Royalty breakdown. Helius always emits this key, as null when
    /// the mint doesn't have royalty data yet.
    #[serde(default)]
    pub royalty: Option<DasRoyalty>,
    /// Supply info (edition counting). Always emitted; null for
    /// plain NFTs.
    #[serde(default)]
    pub supply: Option<DasSupply>,
    /// Token info (supply + decimals + token program + ATA). Helius
    /// populates this for ID-queries + by-owner/by-creator searches.
    /// Skip-on-None since plain `searchAssets` on collection doesn't
    /// always return it for every item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_info: Option<DasTokenInfo>,
}

/// Subset of the SPL Token `Mint` account surfaced by Helius under
/// `token_info`. Supply + decimals are the interesting fields;
/// `token_program` distinguishes SPL Token from Token-2022;
/// `associated_token_address` is the holder's ATA.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasTokenInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supply: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub associated_token_address: Option<String>,
    /// Mint authority for the token. Often `null` for NFTs whose
    /// authority was removed post-mint (the "lock" step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mint_authority: Option<String>,
    /// Freeze authority for the token. Typically `null` for most NFTs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_authority: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasContent {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub json_uri: String,
    pub metadata: DasMetadata,
    pub links: DasLinks,
    pub files: Vec<DasFile>,
    /// File-type category Helius classifies the asset under —
    /// `"image"`, `"video"`, `"audio"`, `"vr"`, or absent. Always
    /// emitted on the wire; skip on `None` to avoid noise when our
    /// local decoder doesn't know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasMetadata {
    pub name: String,
    pub symbol: String,
    pub description: String,
    /// NFT traits / attributes array. Helius populates it from the
    /// off-chain metadata JSON. Omitted when the JSON doesn't have
    /// attributes or we haven't fetched off-chain yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Vec<DasAttribute>>,
    /// MPL token standard — "NonFungible", "ProgrammableNonFungible",
    /// "Fungible", etc. Distinct from the top-level `interface`
    /// string; Helius populates both for backwards compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_standard: Option<String>,
}

/// One NFT trait. Shape matches the Metaplex off-chain JSON spec.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasAttribute {
    pub trait_type: String,
    /// Value can be string, number, or bool in the off-chain JSON;
    /// we keep the raw value for fidelity.
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasLinks {
    pub image: Option<String>,
    pub animation_url: Option<String>,
    /// Project homepage / external URL from the off-chain metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasFile {
    pub uri: String,
    pub mime: String,
    /// CDN-cached URL that Helius resolves and surfaces. Absent when
    /// we produce a file entry locally (cNFT path) without Helius
    /// enrichment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdn_uri: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasAuthority {
    pub address: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasCreator {
    pub address: String,
    pub share: u8,
    pub verified: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasOwnership {
    pub frozen: bool,
    /// Helius-era flag for Token-2022's NonTransferable extension.
    /// Always emitted, defaulting to `false` when parsing pre-flag
    /// responses. (Seen on cNFTs + Token-2022 in real traffic.)
    #[serde(default)]
    pub non_transferable: bool,
    pub delegated: bool,
    pub ownership_model: String,
    pub owner: String,
    /// Delegate wallet. Helius always emits this key, as `null`
    /// when the asset isn't delegated.
    #[serde(default)]
    pub delegate: Option<String>,
}

/// Royalty breakdown Helius surfaces at the top-level `royalty`
/// field. We don't compute it — when present, it was decoded
/// upstream.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasRoyalty {
    pub royalty_model: String,
    pub target: Option<String>,
    pub percent: f64,
    pub basis_points: u32,
    pub primary_sale_happened: bool,
    pub locked: bool,
}

/// Supply metadata for editions. Helius returns this for
/// Master/Edition mints; null for plain NFTs.
///
/// `print_max_supply`, `print_current_supply`, and `edition_nonce`
/// always emit — Helius sends them even when null/zero, and round-
/// tripping without the key is a silent drift. The edition-specific
/// fields (`edition_number`, `master_edition_mint`) genuinely don't
/// appear on non-edition mints; keep them opt-in via skip.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasSupply {
    #[serde(default)]
    pub print_max_supply: Option<u64>,
    #[serde(default)]
    pub print_current_supply: Option<u64>,
    #[serde(default)]
    pub edition_nonce: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_edition_mint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasGrouping {
    pub group_key: String,
    pub group_value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasCompression {
    pub eligible: bool,
    pub compressed: bool,
    pub data_hash: String,
    pub creator_hash: String,
    /// Bubblegum V2 additions. Always emitted — default to empty
    /// strings / zero when the underlying asset is pre-V2 so the
    /// key set matches Helius regardless of tree age.
    #[serde(default)]
    pub collection_hash: String,
    #[serde(default)]
    pub asset_data_hash: String,
    #[serde(default)]
    pub flags: u64,
    pub asset_hash: String,
    pub tree: String,
    pub seq: u64,
    pub leaf_id: u64,
}

/// Response shape for `getAssetProof` + each entry in
/// `getAssetProofBatch`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasAssetProof {
    pub root: String,
    pub proof: Vec<String>,
    pub node_index: u64,
    pub leaf: String,
    pub tree_id: String,
}

// ─── Editions (getNftEditions) ──────────────────────────────────────

/// Summary of a master edition: supply + max_supply at the time we
/// indexed it. `master_edition_pda` is the Metaplex "edition" PDA
/// under the master mint — `["metadata", TM_ID, master_mint, "edition"]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterEditionRecord {
    pub master_mint: String,
    pub master_edition_pda: String,
    pub supply: u64,
    pub max_supply: Option<u64>,
}

/// One print edition derived from a master. The print's own metadata
/// is indexed separately (as a normal DasAsset) when that mint is
/// fetched; this record only captures the parent→child relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintEditionRecord {
    pub print_mint: String,
    pub print_edition_pda: String,
    /// Master edition PDA, not master mint — that's what Edition
    /// accounts actually store in their `parent` field.
    pub parent_master_edition_pda: String,
    pub edition_num: u64,
}

/// Helius's `getNftEditions` response shape. Pagination is 1-indexed to
/// match their public API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasNftEditions {
    pub total: u64,
    pub limit: u64,
    pub page: u64,
    pub master_edition_address: String,
    pub supply: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_supply: Option<u64>,
    pub editions: Vec<DasNftEditionEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasNftEditionEntry {
    pub mint: String,
    pub edition_address: String,
    pub edition: u64,
}

// ─── Token accounts (getTokenAccounts) ──────────────────────────────

/// One SPL Token / Token-2022 account reshaped to Helius's wire form.
/// Raw RPC responses are `getTokenAccountsByOwner`-shaped (base64 or
/// jsonParsed); the fields here are the subset `helius.das`
/// surfaces to callers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasTokenAccount {
    pub address: String,
    pub mint: String,
    pub owner: String,
    pub amount: u64,
    pub delegated_amount: u64,
    pub frozen: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegate: Option<String>,
}

/// `getTokenAccounts` response shape. Pagination is 1-indexed; Helius
/// also supports a cursor variant for deep pagination that we don't
/// mirror here (out of scope — users needing deep cursors should call
/// `getProgramAccountsV2` directly when that ships).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DasTokenAccounts {
    pub total: u64,
    pub limit: u64,
    pub page: u64,
    pub token_accounts: Vec<DasTokenAccount>,
}

// ─── Wallet balances (getBalances) ──────────────────────────────────

/// One token position for `getBalances`. `amount` is the raw on-chain
/// u64; `decimals` comes from the jsonParsed tokenAmount envelope.
/// Field names mirror Helius's REST shape exactly (camelCase).
///
/// `priceInUSD` and `totalPrice` are paid-tier enrichments Helius may
/// return alongside the raw balance. We never populate them (no local
/// price feed) but accept + round-trip them so clients reading real-
/// Helius fixtures and Tidepool outputs get the same key set. They're
/// skipped on serialize when absent to keep our output shape tight.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DasTokenBalance {
    pub token_account: String,
    pub mint: String,
    pub amount: u64,
    pub decimals: u8,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "priceInUSD")]
    pub price_in_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_price: Option<f64>,
}

/// `getBalances` response shape. Matches Helius's REST wire format:
/// `{ tokens: [...], nativeBalance: <lamports> }`. `nativeBalance` is
/// a plain u64 — Helius once returned a nested object with USD pricing
/// but the live REST response is scalar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DasBalances {
    pub tokens: Vec<DasTokenBalance>,
    pub native_balance: u64,
}
