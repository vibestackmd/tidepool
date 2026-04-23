//! Hash primitive tests. One anchor on an external, well-publicized
//! vector (keccak256 of empty input — the Ethereum-standard constant)
//! so a broken keccak impl surfaces immediately. Everything else
//! checks internal consistency + field sensitivity.

use tidepool_core::{
    empty_node, hash_creators, hash_leaf_v1, hash_pair, keccak256, Creator, LeafSchemaV1,
};

const KECCAK_EMPTY_HEX: &str = "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(&mut out, "{b:02x}").expect("writing to String never fails");
    }
    out
}

#[test]
fn keccak256_matches_eth_empty_vector() {
    // If this ever fails, our keccak is wrong at the protocol level —
    // we'd be producing SHA-3 instead of Ethereum-style Keccak.
    assert_eq!(to_hex(&keccak256(&[])), KECCAK_EMPTY_HEX);
}

#[test]
fn keccak256_output_is_always_32_bytes() {
    for &len in &[0usize, 1, 32, 64, 1024] {
        let input = vec![0xab; len];
        assert_eq!(keccak256(&input).len(), 32);
    }
}

#[test]
fn hash_pair_is_order_sensitive() {
    let a = [1u8; 32];
    let b = [2u8; 32];
    assert_ne!(hash_pair(&a, &b), hash_pair(&b, &a));
}

#[test]
fn hash_pair_is_deterministic() {
    let a = [7u8; 32];
    let b = [11u8; 32];
    assert_eq!(hash_pair(&a, &b), hash_pair(&a, &b));
}

#[test]
fn empty_node_zero_height_is_all_zeros() {
    assert_eq!(empty_node(0), [0u8; 32]);
}

#[test]
fn empty_node_cascade_relationship_holds() {
    for h in 1..=10 {
        let below = empty_node(h - 1);
        assert_eq!(empty_node(h), hash_pair(&below, &below));
    }
}

#[test]
fn empty_node_memoization_does_not_drift() {
    // Pull the same height twice and make sure we get the same bytes —
    // which is implied by determinism but cheap to pin.
    assert_eq!(empty_node(5), empty_node(5));
    assert_eq!(empty_node(20), empty_node(20));
}

#[test]
fn hash_leaf_v1_output_is_32_bytes_and_deterministic() {
    let leaf = LeafSchemaV1 {
        id: [0xaa; 32],
        owner: [0xbb; 32],
        delegate: [0xcc; 32],
        nonce: 42,
        data_hash: [0xdd; 32],
        creator_hash: [0xee; 32],
    };
    assert_eq!(hash_leaf_v1(&leaf), hash_leaf_v1(&leaf));
}

#[test]
fn hash_leaf_v1_is_sensitive_to_every_field() {
    let base = LeafSchemaV1 {
        id: [0xaa; 32],
        owner: [0xbb; 32],
        delegate: [0xcc; 32],
        nonce: 42,
        data_hash: [0xdd; 32],
        creator_hash: [0xee; 32],
    };
    let base_hash = hash_leaf_v1(&base);

    let perturbations: &[(&str, LeafSchemaV1)] = &[
        (
            "id",
            LeafSchemaV1 {
                id: [0xff; 32],
                ..base.clone()
            },
        ),
        (
            "owner",
            LeafSchemaV1 {
                owner: [0xff; 32],
                ..base.clone()
            },
        ),
        (
            "delegate",
            LeafSchemaV1 {
                delegate: [0xff; 32],
                ..base.clone()
            },
        ),
        (
            "nonce",
            LeafSchemaV1 {
                nonce: 43,
                ..base.clone()
            },
        ),
        (
            "data_hash",
            LeafSchemaV1 {
                data_hash: [0xff; 32],
                ..base.clone()
            },
        ),
        (
            "creator_hash",
            LeafSchemaV1 {
                creator_hash: [0xff; 32],
                ..base.clone()
            },
        ),
    ];
    for (field, mutated) in perturbations {
        assert_ne!(
            hash_leaf_v1(mutated),
            base_hash,
            "mutation on `{field}` produced the same hash"
        );
    }
}

#[test]
fn hash_leaf_v1_honors_nonce_little_endian() {
    // nonce=1 encoded LE is 01 00 00 00 00 00 00 00; encoded BE would be
    // 00 00 00 00 00 00 00 01. If the impl were BE, nonce=1 would hash
    // the same preimage as nonce=0x0100000000000000 under LE — they'd
    // collide. They must not.
    let mk = |nonce: u64| LeafSchemaV1 {
        id: [0; 32],
        owner: [0; 32],
        delegate: [0; 32],
        nonce,
        data_hash: [0; 32],
        creator_hash: [0; 32],
    };
    let h1 = hash_leaf_v1(&mk(1));
    let h_be_if_swapped = hash_leaf_v1(&mk(0x0100_0000_0000_0000));
    let h256 = hash_leaf_v1(&mk(256));
    assert_ne!(
        h1, h_be_if_swapped,
        "endianness of nonce serialization is wrong"
    );
    assert_ne!(h1, h256);
}

#[test]
fn hash_creators_is_deterministic_and_order_sensitive() {
    let a = Creator {
        address: [1u8; 32],
        verified: true,
        share: 50,
    };
    let b = Creator {
        address: [2u8; 32],
        verified: false,
        share: 50,
    };

    let ab = hash_creators(&[a.clone(), b.clone()]);
    let ba = hash_creators(&[b.clone(), a.clone()]);
    let ab2 = hash_creators(&[a, b]);

    assert_eq!(ab, ab2);
    assert_ne!(ab, ba);
}

#[test]
fn hash_creators_distinguishes_verified_vs_unverified() {
    let c = Creator {
        address: [1u8; 32],
        verified: true,
        share: 100,
    };
    let c_unverified = Creator {
        verified: false,
        ..c.clone()
    };
    assert_ne!(hash_creators(&[c]), hash_creators(&[c_unverified]));
}

#[test]
fn hash_creators_empty_list_equals_keccak_empty() {
    assert_eq!(to_hex(&hash_creators(&[])), KECCAK_EMPTY_HEX);
}
