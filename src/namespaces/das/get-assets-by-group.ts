// helius.das.getAssetsByGroup — returns assets matching a (groupKey,
// groupValue) pair. For MplCore today this is effectively "assets in a
// collection", since MplCore's only grouping is `collection → pubkey`.
// LOCAL_INDEX: only previously-fetched assets are visible.

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";
import { paginate, type PageParams } from "./pagination.js";

interface GetAssetsByGroupParams extends PageParams {
  groupKey: string;
  groupValue: string;
}

export const getAssetsByGroup: Handler = async (ctx, params, id) => {
  const p = params as GetAssetsByGroupParams;
  const items = await ctx.cache.getAssetsByGroup(p.groupKey, p.groupValue);
  return jsonRpcResult(id, paginate(items, p));
};
