//! Merkle proof computation and verification for Tidepool cNFT trees.
//!
//! Standard binary-merkle: at each level, emit the sibling hash, fold
//! `(self, sibling)` or `(sibling, self)` depending on which side
//! we're on, repeat to the root. Unpopulated positions use the empty
//! cascade so we don't pay for zero-filled subtrees.
//!
//! Runtime is O(depth * populated_leaves). Good enough for depth-30
//! trees with tens of thousands of leaves. Above that, a persistent
//! node-level cache wins — future optimization, not v1 concern.

use std::collections::{BTreeMap, BTreeSet};

use crate::hash::{empty_node, hash_pair};
use crate::types::{MerkleProof, TreeState};

/// Errors compute_proof can surface. Out-of-band inputs (bad depth,
/// bad leaf index) — never triggered by on-chain data in practice, but
/// returned as `Result` rather than panicking so callers can handle
/// library-misuse cases gracefully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofError {
    /// Tree depth must be between 1 and 30 inclusive.
    UnsupportedDepth(u8),
    /// Leaf index beyond the tree's capacity (`2^depth`).
    OutOfRange {
        leaf_index: u64,
        depth: u8,
        capacity: u64,
    },
}

impl std::fmt::Display for ProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedDepth(d) => {
                write!(f, "unsupported tree depth {d} (must be 1..=30)")
            }
            Self::OutOfRange {
                leaf_index,
                depth,
                capacity,
            } => write!(
                f,
                "leaf_index {leaf_index} out of range for depth {depth} (capacity {capacity})"
            ),
        }
    }
}

impl std::error::Error for ProofError {}

/// Compute a merkle proof for the leaf at `leaf_index` in `tree`.
///
/// Returns the proof path (sibling-of-leaf up to sibling-of-root), the
/// computed root, the leaf hash at that position (or empty-node(0) if
/// absent), and the node_index = `2^depth + leaf_index` used by DAS's
/// `getAssetProof` wire format.
pub fn compute_proof(tree: &TreeState, leaf_index: u64) -> Result<MerkleProof, ProofError> {
    if tree.depth < 1 || tree.depth > 30 {
        return Err(ProofError::UnsupportedDepth(tree.depth));
    }
    let depth = tree.depth;
    let capacity = 1u64 << depth;
    if leaf_index >= capacity {
        return Err(ProofError::OutOfRange {
            leaf_index,
            depth,
            capacity,
        });
    }

    // Working level: positions present at this height. Start at leaves.
    let mut level: BTreeMap<u64, [u8; 32]> = tree.leaves.clone();
    let mut proof: Vec<[u8; 32]> = Vec::with_capacity(depth as usize);
    let mut current_index = leaf_index;

    for h in 0..depth {
        let sib_idx = current_index ^ 1;
        let sibling = level
            .get(&sib_idx)
            .copied()
            .unwrap_or_else(|| empty_node(h as usize));
        proof.push(sibling);

        // Fold into the next level. Only compute parents whose
        // children include at least one populated position; everything
        // else is implicitly the empty cascade at the next height.
        let mut next: BTreeMap<u64, [u8; 32]> = BTreeMap::new();
        let mut seen: BTreeSet<u64> = BTreeSet::new();
        for &pos in level.keys() {
            let parent = pos >> 1;
            if !seen.insert(parent) {
                continue;
            }
            let left_idx = parent << 1;
            let right_idx = left_idx + 1;
            let left = level
                .get(&left_idx)
                .copied()
                .unwrap_or_else(|| empty_node(h as usize));
            let right = level
                .get(&right_idx)
                .copied()
                .unwrap_or_else(|| empty_node(h as usize));
            next.insert(parent, hash_pair(&left, &right));
        }
        level = next;
        current_index >>= 1;
    }

    // After `depth` folds, `level` has at most one entry at position 0.
    // If the tree was entirely empty, `level` is empty and the root
    // defaults to the empty cascade at `depth`.
    let root = level
        .get(&0)
        .copied()
        .unwrap_or_else(|| empty_node(depth as usize));

    let leaf = tree
        .leaves
        .get(&leaf_index)
        .copied()
        .unwrap_or_else(|| empty_node(0));

    let node_index = (1u64 << depth) + leaf_index;

    Ok(MerkleProof {
        leaf,
        proof,
        root,
        node_index,
    })
}

/// Verify a proof bottom-up. The lowest bit of `leaf_index` at each
/// level says which side of the parent the current node is on.
#[must_use]
pub fn verify_proof(leaf: &[u8; 32], proof: &[[u8; 32]], leaf_index: u64, root: &[u8; 32]) -> bool {
    let mut current = *leaf;
    let mut idx = leaf_index;
    for sibling in proof {
        current = if idx & 1 == 0 {
            hash_pair(&current, sibling)
        } else {
            hash_pair(sibling, &current)
        };
        idx >>= 1;
    }
    &current == root
}
