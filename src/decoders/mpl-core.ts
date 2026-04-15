// MplCore account decoder.
//
// Thin adapter over the Codama-generated Kit-native decoders at
// `src/generated/mpl-core/`. We import `getAssetV1Decoder` and
// `getCollectionV1Decoder` from there — those come from the pinned IDL
// at `idls/mpl_core.json` (see `idls/mpl_core.source.json` for the
// upstream commit). Re-regenerate with `make update-idl`.
//
// The previous hand-rolled Borsh reader lived here before v0.2.1. It
// worked for the base AssetV1 / CollectionV1 header but had no way to
// track upstream layout changes. Switching to Codama means:
//   - Base layout stays current with mpl-core via a one-command refresh
//   - ~200 LOC of byte-reading code is gone
//   - The DasAsset shape this file emits is unchanged — v0.1/v0.2
//     consumers see byte-for-byte identical responses
//
// Plugin parsing (VerifiedCreators, Royalties, Attributes, etc.) is
// still a v0.3 concern. It's not just "add more codecs" — plugins live
// at explicit byte offsets behind a PluginHeaderV1 + PluginRegistryV1
// pair, so it needs a small walker. The IDL has all the types; the
// walker is the future work.

import type { AccountDecoder, DasAsset } from "./index.js";
import { getAssetV1Decoder } from "../generated/mpl-core/accounts/assetV1.js";
import { getCollectionV1Decoder } from "../generated/mpl-core/accounts/collectionV1.js";
import type { UpdateAuthority } from "../generated/mpl-core/types/updateAuthority.js";

export const MPL_CORE_PROGRAM_ID = "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d";

const KEY_ASSET_V1 = 1;
const KEY_COLLECTION_V1 = 5;

/** Fetch off-chain JSON metadata. Supports http(s) and file:// URIs. */
async function fetchMetadata(uri: string): Promise<Record<string, unknown> | null> {
  if (!uri) return null;
  try {
    if (uri.startsWith("file://")) {
      const { readFileSync } = await import("node:fs");
      return JSON.parse(readFileSync(uri.replace("file://", ""), "utf-8"));
    }
    const resp = await fetch(uri, { signal: AbortSignal.timeout(3000) });
    if (!resp.ok) return null;
    return (await resp.json()) as Record<string, unknown>;
  } catch {
    return null;
  }
}

// Map a decoded UpdateAuthority variant into the DAS `authorities` field.
// - None        → no entry (empty authorities array)
// - Address     → direct update authority, scope "full"
// - Collection  → the collection pubkey under scope "collection"; real
//                 authority is inherited from the collection and can't
//                 be resolved without a second account read
function authoritiesFromUpdateAuthority(
  ua: UpdateAuthority,
): Array<{ address: string; scopes: string[] }> {
  if (ua.__kind === "Address") {
    return [{ address: ua.fields[0] as string, scopes: ["full"] }];
  }
  if (ua.__kind === "Collection") {
    return [{ address: ua.fields[0] as string, scopes: ["collection"] }];
  }
  return [];
}

function groupingFromUpdateAuthority(
  ua: UpdateAuthority,
): Array<{ group_key: string; group_value: string }> {
  if (ua.__kind === "Collection") {
    return [{ group_key: "collection", group_value: ua.fields[0] as string }];
  }
  return [];
}

function buildDasAsset(
  id: string,
  iface: "MplCoreAsset" | "MplCoreCollection",
  name: string,
  uri: string,
  owner: string,
  authorities: Array<{ address: string; scopes: string[] }>,
  grouping: Array<{ group_key: string; group_value: string }>,
  metadata: Record<string, unknown> | null,
): DasAsset {
  const props = (metadata?.properties ?? {}) as Record<string, unknown>;
  const files = (props.files ?? []) as Array<{ uri?: string; type?: string }>;

  return {
    id,
    interface: iface,
    content: {
      $schema: "https://schema.metaplex.com/nft/1.0",
      json_uri: uri,
      metadata: {
        name,
        symbol: (metadata?.symbol as string) ?? "",
        description: (metadata?.description as string) ?? "",
      },
      links: {
        image: (metadata?.image as string) ?? null,
        animation_url: (metadata?.animation_url as string) ?? null,
      },
      files: files.map((f) => ({ uri: f.uri ?? "", mime: f.type ?? "" })),
    },
    authorities,
    ownership: {
      frozen: false,
      delegated: false,
      ownership_model: "single",
      owner,
    },
    grouping,
    mutable: true,
    burnt: false,
  };
}

// Reuse the decoders across calls — they're pure and don't hold state.
const assetV1Decoder = getAssetV1Decoder();
const collectionV1Decoder = getCollectionV1Decoder();

export const mplCoreDecoder: AccountDecoder = {
  programId: MPL_CORE_PROGRAM_ID,
  name: "mpl-core",

  async decode(pubkey, data) {
    if (data.length === 0) return null;
    const key = data[0];

    try {
      if (key === KEY_ASSET_V1) {
        const parsed = assetV1Decoder.decode(data);
        const metadata = await fetchMetadata(parsed.uri);

        return buildDasAsset(
          pubkey,
          "MplCoreAsset",
          parsed.name,
          parsed.uri,
          parsed.owner as string,
          authoritiesFromUpdateAuthority(parsed.updateAuthority),
          groupingFromUpdateAuthority(parsed.updateAuthority),
          metadata,
        );
      }

      if (key === KEY_COLLECTION_V1) {
        const parsed = collectionV1Decoder.decode(data);
        const metadata = await fetchMetadata(parsed.uri);

        return buildDasAsset(
          pubkey,
          "MplCoreCollection",
          parsed.name,
          parsed.uri,
          parsed.updateAuthority as string,
          [{ address: parsed.updateAuthority as string, scopes: ["full"] }],
          [],
          metadata,
        );
      }
    } catch (err) {
      console.error(`[mpl-core] Failed to decode ${pubkey}: ${(err as Error).message}`);
    }

    return null;
  },
};
