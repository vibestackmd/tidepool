//! Merkle proof tests. Build small trees by hand, compute proofs with
//! our prover, verify them with our verifier, cross-check roots
//! against a naive recursive computation.

use std::collections::BTreeMap;

use tidepool_rpc_core::{
    compute_proof, empty_node, hash_pair, verify_proof, MerkleProof, ProofError, TreeState,
};

fn leaf(seed: u8) -> [u8; 32] {
    [seed; 32]
}

/// Naive root computation: expand the tree to full leaf capacity,
/// fold pairwise up. If this disagrees with compute_proof's root, one
/// of them is wrong.
fn naive_root(tree: &TreeState) -> [u8; 32] {
    let capacity: usize = 1usize << tree.depth;
    let mut layer: Vec<[u8; 32]> = (0..capacity)
        .map(|i| {
            tree.leaves
                .get(&(i as u64))
                .copied()
                .unwrap_or_else(|| empty_node(0))
        })
        .collect();
    for _ in 0..tree.depth {
        layer = layer
            .chunks(2)
            .map(|pair| hash_pair(&pair[0], &pair[1]))
            .collect();
    }
    layer[0]
}

fn verify(p: &MerkleProof, leaf_index: u64) -> bool {
    verify_proof(&p.leaf, &p.proof, leaf_index, &p.root)
}

#[test]
fn empty_tree_root_equals_empty_cascade_at_depth() {
    let tree = TreeState::new(4);
    let p = compute_proof(&tree, 5).expect("compute");
    assert_eq!(p.root, empty_node(4));
    assert_eq!(p.leaf, empty_node(0));
    assert_eq!(p.proof.len(), 4);
    assert_eq!(p.node_index, 16 + 5);
    assert!(verify(&p, 5));
}

#[test]
fn single_leaf_tree_verifies() {
    let mut leaves = BTreeMap::new();
    leaves.insert(3, leaf(1));
    let tree = TreeState { depth: 4, leaves };
    let p = compute_proof(&tree, 3).expect("compute");
    assert!(verify(&p, 3));
    assert_eq!(p.root, naive_root(&tree));
}

#[test]
fn dense_tree_every_leaf_verifies() {
    let depth = 4;
    let mut leaves = BTreeMap::new();
    for i in 0..16 {
        leaves.insert(i, leaf((i + 1) as u8));
    }
    let tree = TreeState { depth, leaves };
    let expected_root = naive_root(&tree);

    for i in 0..16u64 {
        let p = compute_proof(&tree, i).expect("compute");
        assert_eq!(p.root, expected_root, "root drift at leaf {i}");
        assert!(verify(&p, i), "proof failed to verify at leaf {i}");
    }
}

#[test]
fn sparse_tree_uses_empty_cascade_for_unset_siblings() {
    let mut leaves = BTreeMap::new();
    leaves.insert(10, leaf(1));
    let tree = TreeState { depth: 5, leaves };

    let p10 = compute_proof(&tree, 10).expect("compute");
    assert_eq!(p10.root, naive_root(&tree));
    assert!(verify(&p10, 10));

    // Sibling of the populated leaf at index 10 is empty.
    let p11 = compute_proof(&tree, 11).expect("compute");
    assert_eq!(p11.proof[0], leaf(1), "sibling at 11 is the populated leaf 10");

    // Far-away leaf has an empty sibling at level 0.
    let p7 = compute_proof(&tree, 7).expect("compute");
    assert_eq!(p7.proof[0], empty_node(0), "sibling at 7 is empty");
}

#[test]
fn tampered_proof_fails_to_verify() {
    let mut leaves = BTreeMap::new();
    leaves.insert(0, leaf(1));
    leaves.insert(5, leaf(2));
    leaves.insert(15, leaf(3));
    let tree = TreeState { depth: 4, leaves };

    let p = compute_proof(&tree, 5).expect("compute");
    assert!(verify(&p, 5));

    // Flip a byte in one proof element → verification must fail.
    let mut bad_proof = p.proof.clone();
    bad_proof[1][0] ^= 0xff;
    assert!(!verify_proof(&p.leaf, &bad_proof, 5, &p.root));

    // Wrong leaf index → must fail.
    assert!(!verify_proof(&p.leaf, &p.proof, 4, &p.root));

    // Wrong root → must fail.
    let mut bad_root = p.root;
    bad_root[0] ^= 0xff;
    assert!(!verify_proof(&p.leaf, &p.proof, 5, &bad_root));
}

#[test]
fn node_index_is_pow2_depth_plus_leaf_index() {
    let tree = TreeState::new(6);
    for &idx in &[0u64, 1, 17, 63] {
        let p = compute_proof(&tree, idx).expect("compute");
        assert_eq!(p.node_index, 64 + idx);
    }
}

#[test]
fn leaf_index_bounds_are_enforced() {
    let tree = TreeState::new(3);
    // Capacity is 2^3 = 8; index 8 is out of range.
    assert!(matches!(
        compute_proof(&tree, 8),
        Err(ProofError::OutOfRange { capacity: 8, .. })
    ));
    assert!(compute_proof(&tree, 7).is_ok());
}

#[test]
fn depth_bounds_are_enforced() {
    let zero_depth = TreeState::new(0);
    assert!(matches!(
        compute_proof(&zero_depth, 0),
        Err(ProofError::UnsupportedDepth(0))
    ));
    let too_deep = TreeState::new(31);
    assert!(matches!(
        compute_proof(&too_deep, 0),
        Err(ProofError::UnsupportedDepth(31))
    ));
}

#[test]
fn depth_20_tree_still_produces_a_correct_proof() {
    // Bubblegum mainnet trees are commonly depth 20 (~1M leaf capacity).
    // We obviously don't fill it, but we exercise the full depth path.
    let depth = 20;
    let mut leaves = BTreeMap::new();
    leaves.insert(123_456, leaf(9));
    leaves.insert(999_999, leaf(10));
    let tree = TreeState { depth, leaves };

    let p = compute_proof(&tree, 123_456).expect("compute");
    assert_eq!(p.proof.len(), depth as usize);
    assert!(verify(&p, 123_456));
}
