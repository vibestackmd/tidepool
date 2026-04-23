//! Compressed-NFT service: event-sourced Bubblegum tree replay,
//! pluggable state store, merkle proof responses.
//!
//! Pure merkle math lives in `tidepool-core`; this module owns the
//! Bubblegum-specific state machine, the event types, and the
//! persistence contracts.

pub mod apply;
pub mod indexer;
pub mod leaf_event;
pub mod parser;
pub mod snapshot;
pub mod sqlite_store;
pub mod store;
pub mod tx_extract;
pub mod types;

pub use apply::{apply_event, derive_asset_id, ApplyError};
pub use indexer::{index_tree, IndexError, IndexTreeOptions, IndexTreeResult};

pub use leaf_event::{
    decode_leaf_schema_event, is_noop_program, DecodedLeafSchema, LeafSchemaEventDecoded,
    MPL_NOOP_PROGRAM_ID, SPL_NOOP_PROGRAM_ID,
};
pub use parser::{parse_bubblegum_instruction, ParseError, BUBBLEGUM_PROGRAM_ID};
pub use snapshot::{
    dump_tree, load_tree, LoadSummary, SnapshotBlob, SnapshotKind, TreeSnapshot,
    SNAPSHOT_FORMAT_VERSION,
};
pub use sqlite_store::SqliteCnftStore;
pub use store::{CnftStore, MemoryCnftStore};
pub use tx_extract::{extract_bubblegum_ixs, ExtractedIx, RpcTransactionResponse};
pub use types::{CnftEvent, LeafRecord, MintMetadata, NoopOverride, TreeInfo};
