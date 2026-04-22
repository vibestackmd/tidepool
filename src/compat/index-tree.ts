// surfpoolHeliusIndexTree — custom JSON-RPC method callers use to
// index a Bubblegum tree on demand. Not part of Helius's API. Meant for
// test-setup ergonomics: a test that wants to assert against a real
// on-chain cNFT tree can invoke this once and let subsequent
// `getAsset` / `getAssetProof` calls serve from the local index.

import type { Address } from "@solana/kit";
import type { Handler } from "../context.js";
import { jsonRpcError, jsonRpcResult } from "../context.js";
import { indexTree } from "../cnft/index.js";

interface IndexTreeParams {
  tree?: string;
  // Optional knobs that mirror IndexTreeOptions. Left off the required
  // surface so the common call is `{ tree }`.
  maxSignatures?: number | null;
  pageSize?: number;
}

export const surfpoolHeliusIndexTree: Handler = async (ctx, params, id) => {
  const p = (params ?? {}) as IndexTreeParams;
  const tree = typeof p.tree === "string" ? p.tree : null;
  if (!tree) {
    return jsonRpcError(id, -32602, "Missing required param: tree");
  }
  try {
    const result = await indexTree(
      { upstream: ctx.upstream, store: ctx.cnft },
      tree as Address,
      {
        maxSignatures: p.maxSignatures,
        pageSize: p.pageSize,
      },
    );
    return jsonRpcResult(id, {
      tree,
      processed: result.processed,
      applied: result.applied,
      skipped: result.skipped,
      newest_signature: result.newestApplied,
    });
  } catch (err) {
    return jsonRpcError(
      id,
      -32000,
      `Index failed: ${err instanceof Error ? err.message : String(err)}`,
    );
  }
};
