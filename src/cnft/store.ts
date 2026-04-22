// CnftStore is the persistence interface for cNFT state. Matches the
// pattern established by CacheStore: one interface, swappable impls.
// Default impl is in-memory (store-memory.ts). A SQLite-backed impl can
// land later without touching handlers or the indexer.
//
// All methods are async so the interface accommodates a persistent
// backend. The in-memory impl just wraps resolved Promises.

import type { Address } from "@solana/kit";
import type { LeafRecord, TreeInfo } from "./types.js";

export interface CnftStore {
  // ─── tree lifecycle ───────────────────────────────────────────────

  putTree(info: TreeInfo): Promise<void>;
  getTree(tree: Address): Promise<TreeInfo | null>;

  /** Bump and return the next leaf index for new mints on this tree. */
  allocLeafIndex(tree: Address): Promise<bigint>;

  // ─── per-leaf state ───────────────────────────────────────────────

  putLeaf(record: LeafRecord): Promise<void>;
  getLeaf(assetId: Address): Promise<LeafRecord | null>;

  /** Positional lookup for proof generation. Returns null if the slot is empty. */
  getLeafByIndex(tree: Address, leafIndex: bigint): Promise<LeafRecord | null>;

  /**
   * Enumerate non-burned leaves for a tree. Used by proof + test paths;
   * order is implementation-defined (memory impl returns insertion order).
   */
  listLeaves(tree: Address): Promise<LeafRecord[]>;

  // ─── indexer bookkeeping ──────────────────────────────────────────

  /**
   * Last signature applied to a tree, for incremental scan. null when
   * the tree has never been indexed.
   */
  getLastSignature(tree: Address): Promise<string | null>;
  setLastSignature(tree: Address, signature: string): Promise<void>;
}
