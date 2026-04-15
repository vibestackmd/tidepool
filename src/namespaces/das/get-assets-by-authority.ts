// helius.das.getAssetsByAuthority — returns assets whose update
// authority matches the given address. For MplCore AssetV1 with an
// Address authority, that's a direct match; for Collection-authority
// assets the "authority" stored is the collection pubkey (scope:
// "collection"), so querying by the collection pubkey returns the
// member assets. LOCAL_INDEX — only previously-fetched assets visible.

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";
import { paginate, type PageParams } from "./pagination.js";

interface GetAssetsByAuthorityParams extends PageParams {
  authorityAddress: string;
}

export const getAssetsByAuthority: Handler = async (ctx, params, id) => {
  const p = params as GetAssetsByAuthorityParams;
  const items = await ctx.cache.getAssetsByAuthority(p.authorityAddress);
  return jsonRpcResult(id, paginate(items, p));
};
