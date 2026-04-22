// In-memory CnftStore. Ships as the default; swap in a persistent backend
// later by implementing the interface in store.ts.
//
// Two secondary indexes live here so reads stay O(1): `byLeafIndex` maps
// (tree, leafIndex) → assetId, and `byTree` tracks the insertion-ordered
// list of assetIds per tree so listLeaves is a deterministic O(n).

import type { Address } from "@solana/kit";
import type { CnftStore } from "./store.js";
import type { LeafRecord, TreeInfo } from "./types.js";

export function createCnftMemoryStore(): CnftStore {
  const trees = new Map<Address, TreeInfo>();
  const leavesByAssetId = new Map<Address, LeafRecord>();
  const leafIndexKey = (tree: Address, idx: bigint) => `${tree}:${idx}`;
  const leavesByPosition = new Map<string, Address>();
  const treeLeafOrder = new Map<Address, Address[]>();
  const lastSig = new Map<Address, string>();

  return {
    async putTree(info) {
      trees.set(info.tree, { ...info });
    },
    async getTree(tree) {
      const t = trees.get(tree);
      return t ? { ...t } : null;
    },
    async allocLeafIndex(tree) {
      const t = trees.get(tree);
      if (!t) throw new Error(`allocLeafIndex: unknown tree ${tree}`);
      const idx = t.numMinted;
      t.numMinted = idx + 1n;
      return idx;
    },

    async putLeaf(record) {
      leavesByAssetId.set(record.assetId, { ...record });
      leavesByPosition.set(leafIndexKey(record.tree, record.leafIndex), record.assetId);
      const list = treeLeafOrder.get(record.tree);
      if (list) {
        if (!list.includes(record.assetId)) list.push(record.assetId);
      } else {
        treeLeafOrder.set(record.tree, [record.assetId]);
      }
    },
    async getLeaf(assetId) {
      const r = leavesByAssetId.get(assetId);
      return r ? { ...r } : null;
    },
    async getLeafByIndex(tree, leafIndex) {
      const assetId = leavesByPosition.get(leafIndexKey(tree, leafIndex));
      if (!assetId) return null;
      const r = leavesByAssetId.get(assetId);
      return r ? { ...r } : null;
    },
    async listLeaves(tree) {
      const ids = treeLeafOrder.get(tree) ?? [];
      return ids
        .map((id) => leavesByAssetId.get(id))
        .filter((r): r is LeafRecord => r !== undefined)
        .map((r) => ({ ...r }));
    },

    async getLastSignature(tree) {
      return lastSig.get(tree) ?? null;
    },
    async setLastSignature(tree, signature) {
      lastSig.set(tree, signature);
    },
  };
}
