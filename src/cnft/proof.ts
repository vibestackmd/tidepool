// Merkle proof computation for cNFT trees. Pure: given a TreeState with
// populated leaves and a leafIndex, emit the sibling path from leaf to
// root. Also exposes `verifyProof` so tests can cross-check.
//
// The algorithm is standard binary-merkle: at each level we emit the
// sibling hash, then fold (self, sibling) — or (sibling, self) depending
// on which side we're on — up to the parent, and repeat. Unpopulated
// positions use the empty-node cascade, so we don't need to store
// zero-filled subtrees.
//
// Runtime is O(depth * 2^depth_untouched) worst case if we naively
// recompute; we cut it to O(depth * filled_leaves) by folding only the
// populated positions per level. Good enough for trees up to depth 30
// with tens of thousands of leaves — above that a persistent node-level
// cache wins, but that's a later optimization.

import { emptyNode, hashPair } from "./hash.js";
import type { MerkleProof, TreeState } from "./types.js";

export function computeProof(
  tree: TreeState,
  leafIndex: bigint,
): MerkleProof {
  if (tree.depth < 1 || tree.depth > 30) {
    throw new Error(`computeProof: unsupported tree depth ${tree.depth}`);
  }
  const capacity = 1n << BigInt(tree.depth);
  if (leafIndex < 0n || leafIndex >= capacity) {
    throw new Error(
      `computeProof: leafIndex ${leafIndex} out of range for depth ${tree.depth} (capacity ${capacity})`,
    );
  }

  // Snapshot the current level as a map from position → node hash. Level
  // 0 is leaves; level `depth` is the root. Positions absent from the
  // map resolve to `emptyNode(level)`.
  let level = new Map<bigint, Uint8Array>(tree.leaves);
  const proof: Uint8Array[] = [];

  let currentIndex = leafIndex;
  for (let h = 0; h < tree.depth; h++) {
    const sibIdx = currentIndex ^ 1n;
    const sibling = level.get(sibIdx) ?? emptyNode(h);
    proof.push(sibling);

    // Fold this level into the next. We only need to compute nodes
    // whose children we've seen — anything else stays "empty" and is
    // derived on demand from emptyNode(h+1). Iterate unique parent
    // indexes that have at least one non-empty child.
    const next = new Map<bigint, Uint8Array>();
    const seenParents = new Set<bigint>();
    for (const pos of level.keys()) {
      const parent = pos >> 1n;
      if (seenParents.has(parent)) continue;
      seenParents.add(parent);
      const leftIdx = parent << 1n;
      const rightIdx = leftIdx + 1n;
      const left = level.get(leftIdx) ?? emptyNode(h);
      const right = level.get(rightIdx) ?? emptyNode(h);
      next.set(parent, hashPair(left, right));
    }
    level = next;
    currentIndex >>= 1n;
  }

  // After `depth` iterations, `level` contains exactly the root at
  // index 0 (or nothing, if the tree is entirely empty — in which case
  // the root is the empty cascade at `depth`).
  const root = level.get(0n) ?? emptyNode(tree.depth);

  const leaf = tree.leaves.get(leafIndex) ?? emptyNode(0);
  const nodeIndex = Number((1n << BigInt(tree.depth)) + leafIndex);

  return { leaf, proof, root, nodeIndex };
}

/**
 * Verify a proof bottom-up. `leafIndex` determines whether we hash
 * (self, sibling) or (sibling, self) at each level — the lowest bit
 * says which side of its parent the current node is on.
 */
export function verifyProof(args: {
  leaf: Uint8Array;
  proof: Uint8Array[];
  leafIndex: bigint;
  root: Uint8Array;
}): boolean {
  let current = args.leaf;
  let idx = args.leafIndex;
  for (const sibling of args.proof) {
    if ((idx & 1n) === 0n) {
      current = hashPair(current, sibling);
    } else {
      current = hashPair(sibling, current);
    }
    idx >>= 1n;
  }
  return eq(current, args.root);
}

function eq(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}
