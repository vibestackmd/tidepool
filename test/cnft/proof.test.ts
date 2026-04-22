// Merkle proof tests. Build small test trees by hand, compute proofs
// with our prover, verify them with our verifier, and cross-check the
// resulting root against a naive hand-rolled recursive computation.

import { test } from "node:test";
import assert from "node:assert/strict";
import { computeProof, verifyProof } from "../../src/cnft/proof.js";
import { emptyNode, hashPair } from "../../src/cnft/hash.js";
import type { TreeState } from "../../src/cnft/types.js";
import type { Address } from "@solana/kit";

const FAKE_TREE = "11111111111111111111111111111111" as Address;

function makeLeaf(seed: number): Uint8Array {
  // Distinct-but-deterministic 32-byte "leaf hashes" for tests. We
  // don't care about domain separation here — these aren't real leaves,
  // just values that make the merkle structure observable.
  return new Uint8Array(32).fill(seed);
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// Naive root computation: walk every leaf position, fold pairwise up.
// If this disagrees with computeProof's root, one of them is wrong.
function naiveRoot(tree: TreeState): Uint8Array {
  const capacity = 1 << tree.depth;
  let layer: Uint8Array[] = [];
  for (let i = 0; i < capacity; i++) {
    layer.push(tree.leaves.get(BigInt(i)) ?? emptyNode(0));
  }
  for (let h = 0; h < tree.depth; h++) {
    const next: Uint8Array[] = [];
    for (let i = 0; i < layer.length; i += 2) {
      next.push(hashPair(layer[i]!, layer[i + 1]!));
    }
    layer = next;
  }
  return layer[0]!;
}

test("empty tree: root = emptyNode(depth), proof verifies the empty leaf", () => {
  const tree: TreeState = { tree: FAKE_TREE, depth: 4, leaves: new Map() };
  const { root, leaf, proof, nodeIndex } = computeProof(tree, 5n);

  assert.equal(hex(root), hex(emptyNode(4)));
  assert.equal(hex(leaf), hex(emptyNode(0)));
  assert.equal(proof.length, 4);
  assert.equal(nodeIndex, 16 + 5);
  assert.ok(verifyProof({ leaf, proof, leafIndex: 5n, root }));
});

test("single-leaf tree: proof verifies", () => {
  const tree: TreeState = {
    tree: FAKE_TREE,
    depth: 4,
    leaves: new Map([[3n, makeLeaf(1)]]),
  };
  const got = computeProof(tree, 3n);

  assert.ok(verifyProof({ leaf: got.leaf, proof: got.proof, leafIndex: 3n, root: got.root }));
  assert.equal(hex(got.root), hex(naiveRoot(tree)));
});

test("dense tree: every leaf has a verifiable proof", () => {
  const depth = 4;
  const leaves = new Map<bigint, Uint8Array>();
  for (let i = 0; i < 16; i++) {
    leaves.set(BigInt(i), makeLeaf(i + 1)); // avoid 0 so leaves differ from empty
  }
  const tree: TreeState = { tree: FAKE_TREE, depth, leaves };
  const rootNaive = hex(naiveRoot(tree));

  for (let i = 0; i < 16; i++) {
    const p = computeProof(tree, BigInt(i));
    assert.equal(hex(p.root), rootNaive, `root mismatch at leaf ${i}`);
    assert.ok(
      verifyProof({ leaf: p.leaf, proof: p.proof, leafIndex: BigInt(i), root: p.root }),
      `proof failed to verify at leaf ${i}`,
    );
  }
});

test("sparse tree: unset siblings use the empty cascade", () => {
  const tree: TreeState = {
    tree: FAKE_TREE,
    depth: 5,
    leaves: new Map([[10n, makeLeaf(1)]]),
  };
  const p = computeProof(tree, 10n);
  assert.equal(hex(p.root), hex(naiveRoot(tree)));
  assert.ok(verifyProof({ leaf: p.leaf, proof: p.proof, leafIndex: 10n, root: p.root }));

  // The sibling of an unset leaf should be emptyNode(0).
  const q = computeProof(tree, 11n);
  assert.equal(hex(q.proof[0]!), hex(makeLeaf(1)), "sibling at leaf 11 is leaf 10");
  const r = computeProof(tree, 7n);
  assert.equal(hex(r.proof[0]!), hex(emptyNode(0)), "sibling at leaf 7 (empty side) is empty");
});

test("tampered proof fails to verify", () => {
  const tree: TreeState = {
    tree: FAKE_TREE,
    depth: 4,
    leaves: new Map([
      [0n, makeLeaf(1)],
      [5n, makeLeaf(2)],
      [15n, makeLeaf(3)],
    ]),
  };
  const p = computeProof(tree, 5n);
  assert.ok(verifyProof({ leaf: p.leaf, proof: p.proof, leafIndex: 5n, root: p.root }));

  // Flip a byte in one proof element — verification must fail.
  const bad = p.proof.map((x) => new Uint8Array(x));
  bad[1]![0] ^= 0xff;
  assert.ok(!verifyProof({ leaf: p.leaf, proof: bad, leafIndex: 5n, root: p.root }));

  // Wrong leaf index — verification must fail.
  assert.ok(!verifyProof({ leaf: p.leaf, proof: p.proof, leafIndex: 4n, root: p.root }));

  // Wrong root — verification must fail.
  const badRoot = new Uint8Array(p.root);
  badRoot[0] ^= 0xff;
  assert.ok(!verifyProof({ leaf: p.leaf, proof: p.proof, leafIndex: 5n, root: badRoot }));
});

test("nodeIndex = 2^depth + leafIndex", () => {
  const tree: TreeState = { tree: FAKE_TREE, depth: 6, leaves: new Map() };
  for (const idx of [0n, 1n, 17n, 63n]) {
    assert.equal(computeProof(tree, idx).nodeIndex, 64 + Number(idx));
  }
});

test("leafIndex bounds are enforced", () => {
  const tree: TreeState = { tree: FAKE_TREE, depth: 3, leaves: new Map() };
  assert.throws(() => computeProof(tree, -1n));
  assert.throws(() => computeProof(tree, 8n)); // capacity is 2^3 = 8
  // Edge: last valid index works.
  assert.doesNotThrow(() => computeProof(tree, 7n));
});

test("depth bounds are enforced", () => {
  assert.throws(() => computeProof({ tree: FAKE_TREE, depth: 0, leaves: new Map() }, 0n));
  assert.throws(() => computeProof({ tree: FAKE_TREE, depth: 31, leaves: new Map() }, 0n));
});

test("depth 20 tree still produces a correct proof (realistic cNFT scale)", () => {
  // Bubblegum mainnet trees are commonly depth 20 (~1M leaves). We
  // obviously don't fill it, but we exercise the depth path.
  const depth = 20;
  const leaves = new Map<bigint, Uint8Array>();
  leaves.set(123_456n, makeLeaf(9));
  leaves.set(999_999n, makeLeaf(10));
  const tree: TreeState = { tree: FAKE_TREE, depth, leaves };

  const p = computeProof(tree, 123_456n);
  assert.equal(p.proof.length, depth);
  assert.ok(verifyProof({ leaf: p.leaf, proof: p.proof, leafIndex: 123_456n, root: p.root }));
});
