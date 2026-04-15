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

/** Record for one discovered Metaplex print edition of a master. */
export interface EditionRecord {
  /** Mint address of the print edition itself. */
  mint: string;
  /** Edition PDA for this print (owned by the Token Metadata program). */
  edition_address: string;
  /** Edition number (1-indexed — matches the on-chain Edition.edition field). */
  edition: number;
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

  // v0.5.1 — Token Metadata print editions. Indexed as a side effect
  // when fetch.ts observes an EditionV1 account during mint-as-id
  // routing. Keyed by the master edition PDA, not the master mint.
  putEdition(masterEditionAddress: string, record: EditionRecord): Promise<void>;
  getEditionsByMaster(masterEditionAddress: string): Promise<EditionRecord[]>;

  close?(): Promise<void>;
}
