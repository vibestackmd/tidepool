// helius.das.getAssetsByOwner — returns assets owned by an address that
// the proxy has seen. LOCAL_INDEX semantics: only assets previously
// fetched via getAsset (or getAssetBatch) are indexed, so owners we've
// never touched return empty. This is the documented tradeoff for a
// local-dev mirror without a global indexer.

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";
import { paginate, type PageParams } from "./pagination.js";

interface GetAssetsByOwnerParams extends PageParams {
  ownerAddress: string;
}

export const getAssetsByOwner: Handler = async (ctx, params, id) => {
  const p = params as GetAssetsByOwnerParams;
  const items = await ctx.cache.getAssetsByOwner(p.ownerAddress);
  return jsonRpcResult(id, paginate(items, p));
};
