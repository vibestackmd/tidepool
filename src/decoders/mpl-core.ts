// MplCore account decoder.
//
// Thin adapter over the Codama-generated Kit-native decoders at
// `src/generated/mpl-core/`. We import `getAssetV1Decoder` and
// `getCollectionV1Decoder` from there — those come from the pinned IDL
// at `idls/mpl_core.json` (see `idls/mpl_core.source.json` for the
// upstream commit). Re-regenerate with `make update-idl`.
//
// As of v0.3, this decoder also walks the plugin registry (via
// mpl-core-plugins.ts) so the emitted DasAsset includes a `creators`
// list merged from the Royalties and VerifiedCreators plugins. The
// base AssetV1 / CollectionV1 structs are decoded here; anything past
// the base struct is handled by the plugin walker.

import type { AccountDecoder, DasAsset } from "./index.js";
import { getCollectionV1Decoder } from "../generated/mpl-core/accounts/collectionV1.js";
import type { UpdateAuthority } from "../generated/mpl-core/types/updateAuthority.js";
import type { Plugin } from "../generated/mpl-core/types/plugin.js";
import { walkAssetV1 } from "./mpl-core-plugins.js";

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

// Merge Royalties.creators (share) and VerifiedCreators.signatures
// (verified flag) into the DAS-shaped creators list. Both plugins are
// optional — a creator can appear in one, the other, both, or neither.
// Addresses present only in Royalties default to verified=false;
// addresses present only in VerifiedCreators default to share=0.
function creatorsFromPlugins(
  plugins: Partial<Record<Plugin["__kind"], Plugin>>,
): Array<{ address: string; share: number; verified: boolean }> {
  const map = new Map<string, { address: string; share: number; verified: boolean }>();

  const royalties = plugins.Royalties;
  if (royalties?.__kind === "Royalties") {
    for (const c of royalties.fields[0].creators) {
      const address = c.address as string;
      map.set(address, { address, share: c.percentage, verified: false });
    }
  }

  const verified = plugins.VerifiedCreators;
  if (verified?.__kind === "VerifiedCreators") {
    for (const sig of verified.fields[0].signatures) {
      const address = sig.address as string;
      const existing = map.get(address);
      if (existing) {
        existing.verified = sig.verified;
      } else {
        map.set(address, { address, share: 0, verified: sig.verified });
      }
    }
  }

  return Array.from(map.values());
}

function buildDasAsset(
  id: string,
  iface: "MplCoreAsset" | "MplCoreCollection",
  name: string,
  uri: string,
  owner: string,
  authorities: Array<{ address: string; scopes: string[] }>,
  creators: Array<{ address: string; share: number; verified: boolean }>,
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
    creators,
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

// CollectionV1 is a simpler, plugin-free path for now. (Collections can
// carry plugins too; handling those is a future pass once we see a real
// collection with plugins during testing.)
const collectionV1Decoder = getCollectionV1Decoder();

export const mplCoreDecoder: AccountDecoder = {
  programId: MPL_CORE_PROGRAM_ID,
  name: "mpl-core",

  async decode(pubkey, data) {
    if (data.length === 0) return null;
    const key = data[0];

    try {
      if (key === KEY_ASSET_V1) {
        const { base, plugins } = walkAssetV1(data);
        const metadata = await fetchMetadata(base.uri);

        return buildDasAsset(
          pubkey,
          "MplCoreAsset",
          base.name,
          base.uri,
          base.owner as string,
          authoritiesFromUpdateAuthority(base.updateAuthority),
          creatorsFromPlugins(plugins),
          groupingFromUpdateAuthority(base.updateAuthority),
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
