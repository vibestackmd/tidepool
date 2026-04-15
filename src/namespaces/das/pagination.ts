// Shared pagination helper for list-returning DAS methods. Helius's DAS
// envelope is consistent across getAssetsByOwner/ByGroup/ByCreator/etc:
// { total, limit, page, items }. We produce that shape identically so
// clients don't need a proxy-specific code path.
//
// Supports page-based pagination only for now; cursor-based ("before"
// and "after") lands when persistent sort keys are available in v0.3+.

import type { DasAsset } from "../../decoders/index.js";

export interface PageParams {
  page?: number;
  limit?: number;
}

export interface PagedResult {
  total: number;
  limit: number;
  page: number;
  items: DasAsset[];
}

const DEFAULT_LIMIT = 1000;
const MAX_LIMIT = 1000;

export function paginate(items: DasAsset[], params: PageParams): PagedResult {
  const page = Math.max(1, params.page ?? 1);
  const requested = params.limit ?? DEFAULT_LIMIT;
  const limit = Math.max(1, Math.min(MAX_LIMIT, requested));
  const start = (page - 1) * limit;
  const paged = items.slice(start, start + limit);

  return {
    total: items.length,
    limit,
    page,
    items: paged,
  };
}
