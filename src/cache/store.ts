// CacheStore is the interface every local index implements. Handlers take
// a CacheStore — not a concrete type — so we can swap the in-memory default
// for SQLite or anything else without touching namespace code.
//
// This surface is minimal on purpose: v0.1 only needs put/get for assets
// plus the search-assets filtering. The interface grows with each version
// that adds handlers needing new index shapes.

import type { DasAsset } from "../decoders/index.js";

export interface SearchAssetsFilter {
  ownerAddress?: string;
  authorityAddress?: string;
  interface?: string;
  grouping?: [string, string];
}

export interface CacheStore {
  putAsset(asset: DasAsset): Promise<void>;
  getAsset(id: string): Promise<DasAsset | null>;
  searchAssets(filter: SearchAssetsFilter): Promise<DasAsset[]>;
  close?(): Promise<void>;
}
