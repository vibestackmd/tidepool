// helius.das.getAssetBatch — batch-fetch up to N assets by id. Mirrors
// getAsset semantics: cache-first, fall back to upstream + decoder, and
// populate cache on success. Returns an array aligned with the input
// order; not-found entries are null.

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";
import { fetchAndCacheAsset } from "./fetch.js";

interface GetAssetBatchParams {
  ids: string[];
}

const MAX_BATCH = 1000;

export const getAssetBatch: Handler = async (ctx, params, id) => {
  const { ids } = params as GetAssetBatchParams;

  if (!Array.isArray(ids) || ids.length === 0) {
    return jsonRpcResult(id, []);
  }
  if (ids.length > MAX_BATCH) {
    return {
      jsonrpc: "2.0",
      id,
      error: {
        code: -32602,
        message: `ids length ${ids.length} exceeds max batch size ${MAX_BATCH}`,
      },
    };
  }

  // Parallel upstream reads. Each call reads fresh account state from
  // Surfpool and repopulates the cache — matches getAsset's always-fresh
  // semantics so batch + single calls return identical data.
  const results = await Promise.all(
    ids.map((assetId) => fetchAndCacheAsset(ctx, assetId)),
  );

  return jsonRpcResult(id, results);
};
