// helius.das.getAsset — read an on-chain account, decode it, return in
// DAS shape. Populates the cache as a side effect, which is how
// searchAssets finds anything without needing a custom "register" method.

import type { Handler } from "../../context.js";
import { jsonRpcError, jsonRpcResult } from "../../context.js";
import { fetchAndCacheAsset } from "./fetch.js";

interface GetAssetParams {
  id: string;
}

export const getAsset: Handler = async (ctx, params, id) => {
  const { id: assetId } = params as GetAssetParams;
  const asset = await fetchAndCacheAsset(ctx, assetId);
  if (!asset) {
    return jsonRpcError(id, -32000, "Asset not found");
  }
  return jsonRpcResult(id, asset);
};
