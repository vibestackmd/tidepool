// helius.das.getAssetProofBatch — thin loop over getAssetProof. Matches
// the real Helius endpoint shape: input array of ids, response map
// keyed by id, with null for ids we can't resolve.

import type { Handler } from "../../context.js";
import { jsonRpcError, jsonRpcResult } from "../../context.js";
import { buildAssetProof, type DasAssetProof } from "./get-asset-proof.js";

interface GetAssetProofBatchParams {
  ids: string[];
}

const MAX_BATCH = 1000;

export const getAssetProofBatch: Handler = async (ctx, params, id) => {
  const { ids } = params as GetAssetProofBatchParams;
  if (!Array.isArray(ids)) {
    return jsonRpcError(id, -32602, "Missing or malformed param: ids");
  }
  if (ids.length > MAX_BATCH) {
    return jsonRpcError(id, -32602, `Batch size exceeds maximum of ${MAX_BATCH}`);
  }

  const result: Record<string, DasAssetProof | null> = {};
  // Parallel — each call reads from the same in-memory store; no
  // upstream roundtrip per id.
  await Promise.all(
    ids.map(async (assetId) => {
      result[assetId] = await buildAssetProof(ctx.cnft, assetId);
    }),
  );
  return jsonRpcResult(id, result);
};
