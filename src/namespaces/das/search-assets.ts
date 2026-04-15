// helius.das.searchAssets — flexible asset search with multiple filter
// dimensions. Reads from the local asset cache populated by prior
// getAsset / getAssetBatch calls. LOCAL_INDEX: assets the proxy has
// never seen are invisible, which is the documented tradeoff.
//
// Supported filter dimensions (as of v0.3):
//   ownerAddress       — exact owner match
//   authorityAddress   — exact update-authority match against the
//                        `authorities` field
//   creatorAddress     — match against the `creators` field, populated
//                        from Royalties + VerifiedCreators plugins
//   interface          — MplCoreAsset / MplCoreCollection / etc
//   grouping           — [groupKey, groupValue]
//   tokenType          — fungible / nonFungible / all
//   compressed         — true | false
//   sortBy             — { sortBy, sortDirection }
//
// Pagination: page + limit (cursor-based pagination will ship with
// v0.3 when we have persistent sort keys).

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";
import type {
  SearchAssetsFilter,
  TokenType,
  SortBy,
  SortDirection,
} from "../../cache/index.js";
import { paginate, type PageParams } from "./pagination.js";

interface SearchAssetsParams extends PageParams {
  ownerAddress?: string;
  authorityAddress?: string;
  creatorAddress?: string;
  interface?: string;
  grouping?: [string, string];
  tokenType?: TokenType;
  compressed?: boolean;
  sortBy?: { sortBy: SortBy; sortDirection?: SortDirection };
}

export const searchAssets: Handler = async (ctx, params, id) => {
  const p = params as SearchAssetsParams;

  const filter: SearchAssetsFilter = {
    ownerAddress: p.ownerAddress,
    authorityAddress: p.authorityAddress,
    creatorAddress: p.creatorAddress,
    interface: p.interface,
    grouping: p.grouping,
    tokenType: p.tokenType,
    compressed: p.compressed,
    sortBy: p.sortBy,
  };

  const items = await ctx.cache.searchAssets(filter);
  return jsonRpcResult(id, paginate(items, p));
};
