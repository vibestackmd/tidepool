// DAS namespace surface. Registers every handler this namespace provides.
// The router merges this map with the other namespaces at startup.

import type { Handler } from "../../context.js";
import { getAsset } from "./get-asset.js";
import { getAssetBatch } from "./get-asset-batch.js";
import { getAssetsByOwner } from "./get-assets-by-owner.js";
import { getAssetsByGroup } from "./get-assets-by-group.js";
import { getAssetsByAuthority } from "./get-assets-by-authority.js";
import { getAssetsByCreator } from "./get-assets-by-creator.js";
import { getNftEditions } from "./get-nft-editions.js";
import { searchAssets } from "./search-assets.js";

export const dasHandlers: Record<string, Handler> = {
  getAsset,
  getAssetBatch,
  getAssetsByOwner,
  getAssetsByGroup,
  getAssetsByAuthority,
  getAssetsByCreator,
  getNftEditions,
  searchAssets,
};
