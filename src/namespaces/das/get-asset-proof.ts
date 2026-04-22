// helius.das.getAssetProof — return the merkle proof for a compressed
// asset. Pulls the leaf record from the cNFT store, materializes the
// tree's current state from all non-burned leaves, runs the pure
// computeProof, and renders everything to base58 for the DAS wire shape.

import { getBase58Decoder, type Address } from "@solana/kit";
import type { Handler } from "../../context.js";
import { jsonRpcError, jsonRpcResult } from "../../context.js";
import { computeProof, type TreeState } from "../../cnft/index.js";

const base58 = getBase58Decoder();

interface GetAssetProofParams {
  id: string;
}

export const getAssetProof: Handler = async (ctx, params, id) => {
  const { id: assetId } = params as GetAssetProofParams;
  if (!assetId) {
    return jsonRpcError(id, -32602, "Missing required param: id");
  }
  const proof = await buildAssetProof(ctx.cnft, assetId);
  if (!proof) {
    return jsonRpcError(id, -32000, "Asset not found or tree not indexed");
  }
  return jsonRpcResult(id, proof);
};

export interface DasAssetProof {
  root: string;
  proof: string[];
  node_index: number;
  leaf: string;
  tree_id: string;
}

export async function buildAssetProof(
  store: import("../../cnft/index.js").CnftStore,
  assetId: string,
): Promise<DasAssetProof | null> {
  const leaf = await store.getLeaf(assetId as Address);
  if (!leaf) return null;
  const tree = await store.getTree(leaf.tree);
  if (!tree) return null;

  const state: TreeState = {
    tree: leaf.tree,
    depth: tree.depth,
    leaves: new Map(),
  };
  for (const r of await store.listLeaves(leaf.tree)) {
    if (!r.burned) state.leaves.set(r.leafIndex, r.leafHash);
  }

  const p = computeProof(state, leaf.leafIndex);
  return {
    root: base58.decode(p.root),
    proof: p.proof.map((s) => base58.decode(s)),
    node_index: p.nodeIndex,
    leaf: base58.decode(p.leaf),
    tree_id: leaf.tree as string,
  };
}
