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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Populated only for Bubblegum cNFTs. Omitted entirely for
    /// uncompressed MplCore / Token Metadata assets so consumers can
    /// tell them apart at a glance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression: Option<DasCompression>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasContent {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub json_uri: String,
    pub metadata: DasMetadata,
    pub links: DasLinks,
    pub files: Vec<DasFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasMetadata {
    pub name: String,
    pub symbol: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasLinks {
    pub image: Option<String>,
    pub animation_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasFile {
    pub uri: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasAuthority {
    pub address: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasCreator {
    pub address: String,
    pub share: u8,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasOwnership {
    pub frozen: bool,
    pub delegated: bool,
    pub ownership_model: String,
    pub owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasGrouping {
    pub group_key: String,
    pub group_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasCompression {
    pub eligible: bool,
    pub compressed: bool,
    pub data_hash: String,
    pub creator_hash: String,
    pub asset_hash: String,
    pub tree: String,
    pub seq: u64,
    pub leaf_id: u64,
}

/// Response shape for `getAssetProof` + each entry in
/// `getAssetProofBatch`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DasAssetProof {
    pub root: String,
    pub proof: Vec<String>,
    pub node_index: u64,
    pub leaf: String,
    pub tree_id: String,
}
