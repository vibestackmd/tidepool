// helius.das.searchAssets — reads from the local asset cache populated by
// prior getAsset calls. Queries against addresses we've never fetched
// return empty, which is the documented local-index behavior (LOCAL_INDEX
// compat level). Real Helius scans their global index; we scan ours.

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";

interface SearchAssetsParams {
  ownerAddress?: string;
  authorityAddress?: string;
  interface?: string;
  grouping?: [string, string];
  page?: number;
  limit?: number;
}

export const searchAssets: Handler = async (ctx, params, id) => {
  const p = params as SearchAssetsParams;

  const items = await ctx.cache.searchAssets({
    ownerAddress: p.ownerAddress,
    authorityAddress: p.authorityAddress,
    interface: p.interface,
    grouping: p.grouping,
  });

  const page = p.page ?? 1;
  const limit = p.limit ?? 20;
  const start = (page - 1) * limit;
  const paged = items.slice(start, start + limit);

  return jsonRpcResult(id, {
    total: items.length,
    limit,
    page,
    items: paged,
  });
};
