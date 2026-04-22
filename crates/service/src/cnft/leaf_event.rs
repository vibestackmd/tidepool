//! LeafSchemaEvent decoder — the authoritative new-state payload
//! Bubblegum emits via inner CPI to the noop program on every
//! leaf-mutating instruction.
//!
//! Wire layout is mpl-bubblegum's `LeafSchemaEvent` struct,
//! Borsh-serialized:
//!
//! ```text
//! event_type  : BubblegumEventType (Borsh enum → u8)
//! version     : Version             (Borsh enum → u8)
//! schema      : LeafSchema          (Borsh enum, V1 or V2)
//! leaf_hash   : [u8; 32]            (final new leaf hash)
//! ```
//!
//! We deserialize the whole struct via mpl-bubblegum's own type
//! definitions — anchor-generated + `BorshDeserialize`-derived, so we
//! inherit any layout changes they ship without hand-maintaining
//! parallel logic. The first-byte fast-path lets us cheaply reject
//! non-LeafSchemaEvent noop payloads (e.g. spl-account-compression
//! ChangeLogEvents) before spending decoder cycles.

use borsh::BorshDeserialize;
use mpl_bubblegum::types::{BubblegumEventType, LeafSchema};
use mpl_bubblegum::LeafSchemaEvent;

/// Two noop program IDs in the wild. SPL Noop has been Bubblegum's
/// historical sink; MPL Noop ships with the V2 ix family.
pub const SPL_NOOP_PROGRAM_ID: &str = "noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV";
pub const MPL_NOOP_PROGRAM_ID: &str = "mnoopTCrg4p8ry25e4bcWA9XZjbNjMTfgYVGGEdRsf3";

/// True when the given program id is one of the two noop sinks.
#[must_use]
pub fn is_noop_program(program_id: &str) -> bool {
    program_id == SPL_NOOP_PROGRAM_ID || program_id == MPL_NOOP_PROGRAM_ID
}

/// Decoded LeafSchemaEvent in our service-layer shape. All Pubkey
/// fields are unpacked to `[u8; 32]` so downstream logic compares
/// against stored state without re-encoding through base58.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafSchemaEventDecoded {
    pub schema: DecodedLeafSchema,
    /// The final leaf hash emitted by Bubblegum — useful to
    /// cross-check our recomputed leaf_hash against on-chain truth.
    pub leaf_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedLeafSchema {
    V1 {
        id: [u8; 32],
        owner: [u8; 32],
        delegate: [u8; 32],
        nonce: u64,
        data_hash: [u8; 32],
        creator_hash: [u8; 32],
    },
    /// V2 variants carry extra fields (collection_hash, asset_data_hash,
    /// flags). We don't track them yet; represented distinctly so the
    /// parser surfaces `Unsupported` rather than silently dropping.
    V2 {
        id: [u8; 32],
        owner: [u8; 32],
        delegate: [u8; 32],
        nonce: u64,
        data_hash: [u8; 32],
        creator_hash: [u8; 32],
    },
}

/// Decode noop CPI data as a LeafSchemaEvent. Returns `None` for:
/// - Non-LeafSchemaEvent noop payloads (wrong event-type byte)
/// - Truncated or malformed bytes
/// - Anything that doesn't Borsh-parse cleanly
///
/// Never panics — this is hot, called once per inner noop CPI per tx.
#[must_use]
pub fn decode_leaf_schema_event(data: &[u8]) -> Option<LeafSchemaEventDecoded> {
    // Cheap discriminator check before decoding. BubblegumEventType's
    // Borsh wire form for LeafSchemaEvent is 0x01; ChangeLogEvent from
    // spl-account-compression has its own layout we want to skip.
    if data.is_empty() || data[0] != BubblegumEventType::LeafSchemaEvent as u8 {
        return None;
    }

    let event = LeafSchemaEvent::try_from_slice(data).ok()?;
    let schema = match event.schema {
        LeafSchema::V1 {
            id,
            owner,
            delegate,
            nonce,
            data_hash,
            creator_hash,
        } => DecodedLeafSchema::V1 {
            id: id.to_bytes(),
            owner: owner.to_bytes(),
            delegate: delegate.to_bytes(),
            nonce,
            data_hash,
            creator_hash,
        },
        LeafSchema::V2 {
            id,
            owner,
            delegate,
            nonce,
            data_hash,
            creator_hash,
            ..
        } => DecodedLeafSchema::V2 {
            id: id.to_bytes(),
            owner: owner.to_bytes(),
            delegate: delegate.to_bytes(),
            nonce,
            data_hash,
            creator_hash,
        },
    };

    Some(LeafSchemaEventDecoded {
        schema,
        leaf_hash: event.leaf_hash,
    })
}

impl LeafSchemaEventDecoded {
    /// Convert into the service-layer `NoopOverride` shape when the
    /// schema is V1. V2 returns None — the parser turns that into a
    /// `ParseError::Unsupported`.
    #[must_use]
    pub fn as_v1_override(&self) -> Option<crate::cnft::types::NoopOverride> {
        match &self.schema {
            DecodedLeafSchema::V1 {
                nonce,
                owner,
                delegate,
                data_hash,
                creator_hash,
                ..
            } => Some(crate::cnft::types::NoopOverride {
                leaf_index: *nonce,
                nonce: *nonce,
                owner: *owner,
                delegate: *delegate,
                data_hash: *data_hash,
                creator_hash: *creator_hash,
            }),
            DecodedLeafSchema::V2 { .. } => None,
        }
    }
}
