//! Token Metadata decoder tests.
//!
//! mpl-token-metadata 5.x pulls solana-program v2, while mpl-core and
//! mpl-bubblegum pull v3 — two `Pubkey` types coexist in the tree.
//! That means we can't trivially construct `Metadata` values in tests
//! using our local solana-program v3 Pubkey. We cover the decoder's
//! boundary behavior here; full round-trip validation happens when
//! the server crate exercises real mainnet-forked Metadata accounts
//! in integration tests.

use tidepool_rpc::das::{AccountDecoder, TokenMetadataDecoder};

#[test]
fn decoder_program_id_and_name() {
    let d = TokenMetadataDecoder;
    assert_eq!(d.program_id(), "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
    assert_eq!(d.name(), "V1_NFT");
}

#[test]
fn empty_data_returns_none() {
    let decoded = TokenMetadataDecoder.decode("pk", &[]).unwrap();
    assert!(decoded.is_none());
}

#[test]
fn key_edition_v1_byte_returns_none() {
    // mpl_token_metadata::types::Key::EditionV1 = 1 (not MetadataV1).
    let decoded = TokenMetadataDecoder.decode("pk", &[1, 2, 3]).unwrap();
    assert!(decoded.is_none());
}

#[test]
fn key_master_edition_v1_byte_returns_none() {
    // MasterEditionV1 = 2 — belongs to getNftEditions, not getAsset.
    let decoded = TokenMetadataDecoder.decode("pk", &[2, 0, 0, 0]).unwrap();
    assert!(decoded.is_none());
}

#[test]
fn uninitialized_byte_returns_none() {
    // Key::Uninitialized = 0 → must not decode.
    let decoded = TokenMetadataDecoder.decode("pk", &[0]).unwrap();
    assert!(decoded.is_none());
}

#[test]
fn malformed_metadata_v1_body_returns_error() {
    // First byte says MetadataV1 (4) but the body is truncated —
    // from_bytes should fail cleanly, we should surface a
    // DecodeFailed error (not a panic, not silent None).
    let mut data = vec![4u8]; // Key::MetadataV1 discriminator
    data.extend_from_slice(&[0; 5]); // too few bytes for a valid Metadata
    let result = TokenMetadataDecoder.decode("pk", &data);
    assert!(result.is_err(), "malformed MetadataV1 should be an explicit error, not None");
}
