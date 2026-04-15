// CacheStore is the interface every local index implements. Handlers take
// a CacheStore — not a concrete type — so we can swap the in-memory default
// for SQLite or anything else without touching namespace code.
//
// This interface grows one method per shipped handler. Every "by X" query
// maps to a secondary index the concrete store maintains on putAsset.

import type { DasAsset } from "../decoders/index.js";

export type SortBy = "created" | "updated" | "recent_action" | "none";
export type SortDirection = "asc" | "desc";
export type TokenType = "fungible" | "nonFungible" | "all";

export interface SearchAssetsFilter {
  ownerAddress?: string;
  authorityAddress?: string;
  creatorAddress?: string;
  interface?: string;
  grouping?: [string, string];
  tokenType?: TokenType;
  compressed?: boolean;
  sortBy?: { sortBy: SortBy; sortDirection?: SortDirection };
}

export interface CacheStore {
  putAsset(asset: DasAsset): Promise<void>;
  getAsset(id: string): Promise<DasAsset | null>;
  getAssetBatch(ids: string[]): Promise<Array<DasAsset | null>>;
  getAssetsByOwner(owner: string): Promise<DasAsset[]>;
  getAssetsByGroup(groupKey: string, groupValue: string): Promise<DasAsset[]>;
  getAssetsByAuthority(authority: string): Promise<DasAsset[]>;
  getAssetsByCreator(creator: string): Promise<DasAsset[]>;
  searchAssets(filter: SearchAssetsFilter): Promise<DasAsset[]>;
  close?(): Promise<void>;
}
