// helius.das.searchAssets — flexible asset search with multiple filter
// dimensions. Reads from the local asset cache populated by prior
// getAsset / getAssetBatch calls. LOCAL_INDEX: assets the proxy has
// never seen are invisible, which is the documented tradeoff.
//
// Supported filter dimensions (in v0.2):
//   ownerAddress       — exact owner match
//   authorityAddress   — exact update-authority match (via the new
//                        `authorities` field, NOT the v0.1 shortcut
//                        that aliased to owner)
//   creatorAddress     — reserved; accepted but returns empty until
//                        plugin parsing lands (v0.3)
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

  // creatorAddress is accepted but not yet implementable — MplCore
  // creators live in plugin data that the current decoder skips. Any
  // creatorAddress filter returns empty so callers can feature-detect
  // cleanly rather than getting wrong results.
  if (p.creatorAddress) {
    return jsonRpcResult(id, paginate([], p));
  }

  const filter: SearchAssetsFilter = {
    ownerAddress: p.ownerAddress,
    authorityAddress: p.authorityAddress,
    interface: p.interface,
    grouping: p.grouping,
    tokenType: p.tokenType,
    compressed: p.compressed,
    sortBy: p.sortBy,
  };

  const items = await ctx.cache.searchAssets(filter);
  return jsonRpcResult(id, paginate(items, p));
};
