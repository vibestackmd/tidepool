// In-memory CacheStore. The v0.1 default — no dependencies, ephemeral,
// survives only for the process lifetime. A SQLite-backed store can slot
// in behind the same interface when persistence matters.

import type { DasAsset } from "../decoders/index.js";
import type { CacheStore, SearchAssetsFilter } from "./store.js";

export function createMemoryCache(): CacheStore {
  const assets = new Map<string, DasAsset>();

  return {
    async putAsset(asset) {
      assets.set(asset.id, asset);
    },

    async getAsset(id) {
      return assets.get(id) ?? null;
    },

    async searchAssets(filter: SearchAssetsFilter) {
      let items = Array.from(assets.values());

      const ownerFilter = filter.ownerAddress ?? filter.authorityAddress;
      if (ownerFilter) {
        items = items.filter((a) => a.ownership.owner === ownerFilter);
      }

      if (filter.interface) {
        items = items.filter((a) => a.interface === filter.interface);
      }

      if (filter.grouping) {
        const [, collectionId] = filter.grouping;
        items = items.filter((a) =>
          a.grouping.some((g) => g.group_value === collectionId),
        );
      }

      return items;
    },
  };
}
