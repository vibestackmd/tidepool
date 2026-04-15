// Shared DAS fetch helper. `getAsset` and any future method that needs to
// materialize an asset from an on-chain address goes through this — hit
// the cache first, then fall back to the upstream account read + decoder
// pipeline, then populate the cache on success.

import type { DasAsset } from "../../decoders/index.js";
import type { RequestContext } from "../../context.js";

export async function fetchAndCacheAsset(
  ctx: RequestContext,
  address: string,
): Promise<DasAsset | null> {
  const account = await ctx.upstream.getAccount(address);
  if (!account) return null;

  const decoder = ctx.decoders.find((d) => d.programId === account.owner);
  if (!decoder) return null;

  const asset = await decoder.decode(address, account.data);
  if (asset) {
    await ctx.cache.putAsset(asset);
  }
  return asset;
}
