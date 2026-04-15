// DAS namespace surface. Registers every handler this namespace provides.
// The router merges this map with the other namespaces at startup.

import type { Handler } from "../../context.js";
import { getAsset } from "./get-asset.js";
import { searchAssets } from "./search-assets.js";

export const dasHandlers: Record<string, Handler> = {
  getAsset,
  searchAssets,
};
