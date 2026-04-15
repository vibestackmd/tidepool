// In-memory CacheStore. Maintains secondary indexes for the "by X"
// queries so those methods stay O(1) lookup + scan of a filtered Set.
// Every putAsset walks the asset fields once and updates every index.

import type { DasAsset } from "../decoders/index.js";
import type { CacheStore, SearchAssetsFilter } from "./store.js";

const COLLECTION_INTERFACES = new Set(["MplCoreCollection"]);

function groupingKey(groupKey: string, groupValue: string): string {
  return `${groupKey}:${groupValue}`;
}

export function createMemoryCache(): CacheStore {
  // Primary store — id → asset.
  const assets = new Map<string, DasAsset>();

  // Secondary indexes: each maps an attribute value to the set of asset
  // ids matching it. Populated on every putAsset, never removed (v0.2
  // assumes assets aren't deleted; add invalidation when burn/transfer
  // handling lands).
  const byOwner = new Map<string, Set<string>>();
  const byGrouping = new Map<string, Set<string>>();
  const byAuthority = new Map<string, Set<string>>();

  function addToIndex(
    index: Map<string, Set<string>>,
    key: string,
    id: string,
  ): void {
    let set = index.get(key);
    if (!set) {
      set = new Set();
      index.set(key, set);
    }
    set.add(id);
  }

  function materialize(ids: Iterable<string>): DasAsset[] {
    const out: DasAsset[] = [];
    for (const id of ids) {
      const asset = assets.get(id);
      if (asset) out.push(asset);
    }
    return out;
  }

  return {
    async putAsset(asset) {
      assets.set(asset.id, asset);

      if (asset.ownership.owner) {
        addToIndex(byOwner, asset.ownership.owner, asset.id);
      }

      for (const g of asset.grouping) {
        addToIndex(
          byGrouping,
          groupingKey(g.group_key, g.group_value),
          asset.id,
        );
      }

      for (const a of asset.authorities) {
        addToIndex(byAuthority, a.address, asset.id);
      }
    },

    async getAsset(id) {
      return assets.get(id) ?? null;
    },

    async getAssetBatch(ids) {
      return ids.map((id) => assets.get(id) ?? null);
    },

    async getAssetsByOwner(owner) {
      const ids = byOwner.get(owner);
      return ids ? materialize(ids) : [];
    },

    async getAssetsByGroup(groupKey, groupValue) {
      const ids = byGrouping.get(groupingKey(groupKey, groupValue));
      return ids ? materialize(ids) : [];
    },

    async getAssetsByAuthority(authority) {
      const ids = byAuthority.get(authority);
      return ids ? materialize(ids) : [];
    },

    async searchAssets(filter: SearchAssetsFilter) {
      // Start from the narrowest index we have. Each filter further
      // refines the candidate set. No filters = full scan.
      let candidates: DasAsset[];

      if (filter.ownerAddress) {
        const ids = byOwner.get(filter.ownerAddress);
        candidates = ids ? materialize(ids) : [];
      } else if (filter.authorityAddress) {
        const ids = byAuthority.get(filter.authorityAddress);
        candidates = ids ? materialize(ids) : [];
      } else if (filter.grouping) {
        const [gk, gv] = filter.grouping;
        const ids = byGrouping.get(groupingKey(gk, gv));
        candidates = ids ? materialize(ids) : [];
      } else {
        candidates = Array.from(assets.values());
      }

      // Secondary predicates. Apply only those not already satisfied by
      // the starting index.
      if (filter.ownerAddress) {
        candidates = candidates.filter(
          (a) => a.ownership.owner === filter.ownerAddress,
        );
      }
      if (filter.authorityAddress) {
        candidates = candidates.filter((a) =>
          a.authorities.some((x) => x.address === filter.authorityAddress),
        );
      }
      if (filter.grouping) {
        const [gk, gv] = filter.grouping;
        candidates = candidates.filter((a) =>
          a.grouping.some((g) => g.group_key === gk && g.group_value === gv),
        );
      }
      if (filter.interface) {
        candidates = candidates.filter((a) => a.interface === filter.interface);
      }

      // tokenType: all MplCore assets are non-fungible. Fungible queries
      // return empty until a fungible decoder ships.
      if (filter.tokenType === "fungible") {
        candidates = [];
      }
      // "nonFungible" and "all" are already satisfied by the MplCore set.

      // compressed: all MplCore assets are non-compressed. Compressed
      // queries return empty until cNFT support ships.
      if (filter.compressed === true) {
        candidates = [];
      }

      // Sort. "created" / "updated" / "recent_action" all require slot
      // metadata we don't track yet, so we treat them as identity sorts
      // over id (stable, deterministic). When transaction indexing lands
      // in v0.3+ we can wire real sort keys.
      const dir = filter.sortBy?.sortDirection ?? "asc";
      candidates.sort((a, b) => {
        const cmp = a.id.localeCompare(b.id);
        return dir === "asc" ? cmp : -cmp;
      });

      // Exclude collection accounts from asset searches by default —
      // mirrors Helius's convention where collections are retrievable
      // via getAsset but don't appear in searchAssets results.
      if (!filter.interface) {
        candidates = candidates.filter(
          (a) => !COLLECTION_INTERFACES.has(a.interface),
        );
      }

      return candidates;
    },
  };
}
