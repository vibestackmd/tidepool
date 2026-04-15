// helius.das.getAssetsByCreator — returns assets where the given address
// appears in the merged creators list. LOCAL_INDEX: only assets the proxy
// has fetched (and whose Royalties or VerifiedCreators plugins reference
// the address) are visible.
//
// Unlocked in v0.3 by the plugin walker — before plugin parsing shipped,
// MplCore creators weren't decoded so this method had no data to query
// against. Still filters in the same way real Helius does: the address
// must be a creator of the asset, regardless of whether it's verified.

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";
import { paginate, type PageParams } from "./pagination.js";

interface GetAssetsByCreatorParams extends PageParams {
  creatorAddress: string;
  onlyVerified?: boolean;
}

export const getAssetsByCreator: Handler = async (ctx, params, id) => {
  const p = params as GetAssetsByCreatorParams;
  let items = await ctx.cache.getAssetsByCreator(p.creatorAddress);

  // `onlyVerified` narrows to signed creator attestations. Addresses only
  // present in Royalties (no VerifiedCreators entry) are filtered out.
  if (p.onlyVerified) {
    items = items.filter((a) =>
      a.creators.some((c) => c.address === p.creatorAddress && c.verified),
    );
  }

  return jsonRpcResult(id, paginate(items, p));
};
