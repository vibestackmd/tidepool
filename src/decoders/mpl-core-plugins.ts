// MplCore plugin walker.
//
// MplCore stores plugins at explicit byte offsets past the base account
// data. The layout is:
//
//   [0 .. baseEnd)            AssetV1 base struct (variable size)
//   [baseEnd .. headerEnd)    PluginHeaderV1 { key, pluginRegistryOffset }
//   [pluginRegistryOffset..)  PluginRegistryV1 { registry[], externalRegistry[] }
//
// Each RegistryRecord in PluginRegistryV1.registry contains an `offset`
// (absolute byte index into the account) where that plugin's data lives,
// plus a `pluginType` discriminant. The Plugin discriminated union's
// encoded form starts at that offset.
//
// Codama's generated decoders expose `.read(bytes, offset)` which returns
// `[value, nextOffset]`, so walking is just a sequence of targeted reads.
//
// Reference (pinned): mpl-core@9afdae25783bcca85b835cfc7bd8e2cd47b3198d
//   programs/mpl-core/src/plugins/plugin_registry.rs
//     RegistryRecord.offset: usize // "The offset to the plugin in the account."
//
// Errors in plugin parsing are swallowed and logged — we degrade to
// "base asset, no plugins" rather than failing the whole decode. A
// corrupt plugin shouldn't take down getAsset for an otherwise valid
// account.

import { getAssetV1Decoder } from "../generated/mpl-core/accounts/assetV1.js";
import { getPluginHeaderV1Decoder } from "../generated/mpl-core/accounts/pluginHeaderV1.js";
import { getPluginRegistryV1Decoder } from "../generated/mpl-core/accounts/pluginRegistryV1.js";
import { getPluginDecoder } from "../generated/mpl-core/types/plugin.js";
import { PluginType } from "../generated/mpl-core/types/pluginType.js";
import type { AssetV1 } from "../generated/mpl-core/accounts/assetV1.js";
import type { Plugin } from "../generated/mpl-core/types/plugin.js";

export interface WalkedAsset {
  base: AssetV1;
  // Plugins keyed by their variant tag (e.g. "VerifiedCreators", "Royalties").
  // A single asset can carry at most one of each plugin type, so a plain
  // object keyed by variant name is sufficient and ergonomic for callers.
  plugins: Partial<Record<Plugin["__kind"], Plugin>>;
}

// Reuse decoders across calls — all pure, no state.
const assetV1Decoder = getAssetV1Decoder();
const pluginHeaderV1Decoder = getPluginHeaderV1Decoder();
const pluginRegistryV1Decoder = getPluginRegistryV1Decoder();
const pluginDecoder = getPluginDecoder();

export function walkAssetV1(data: Uint8Array): WalkedAsset {
  // Base struct first. `.read(bytes, 0)` returns [value, byteOffsetAfter].
  // If the account has trailing data beyond the base struct, that's where
  // PluginHeaderV1 begins.
  const [base, baseEnd] = assetV1Decoder.read(data, 0);

  // No trailing bytes → no plugins. Common case for assets minted without
  // a plugin set.
  if (baseEnd >= data.length) {
    return { base, plugins: {} };
  }

  const plugins: Partial<Record<Plugin["__kind"], Plugin>> = {};

  try {
    // Read PluginHeaderV1 immediately after the base struct. The header
    // tells us where the registry lives — registries are stored at the
    // end of the account so plugins can be appended without shifting
    // existing data.
    const [header] = pluginHeaderV1Decoder.read(data, baseEnd);
    const registryOffset = Number(header.pluginRegistryOffset);

    if (registryOffset >= data.length) {
      // Header says registry is past end-of-account. Corrupt, bail.
      return { base, plugins };
    }

    const [registry] = pluginRegistryV1Decoder.read(data, registryOffset);

    // Decode each plugin at its absolute byte offset. We intentionally
    // don't sort — each offset is independent of the others, so order
    // doesn't matter for decoding. External plugins (external_registry)
    // are a separate surface and we skip them; the app-visible DAS fields
    // (creators, royalties, attributes) all live in the core registry.
    for (const record of registry.registry) {
      const offset = Number(record.offset);
      if (offset >= data.length) continue;
      try {
        const [plugin] = pluginDecoder.read(data, offset);
        plugins[plugin.__kind] = plugin;
      } catch (err) {
        const typeName = PluginType[record.pluginType] ?? "?";
        console.error(
          `[mpl-core] Failed to decode plugin ${typeName} at offset ${offset}: ${(err as Error).message}`,
        );
      }
    }
  } catch (err) {
    // Header or registry decode failed — return base asset only. The
    // caller gets a usable asset, just without the plugin-derived fields.
    console.error(
      `[mpl-core] Failed to walk plugins at offset ${baseEnd}: ${(err as Error).message}`,
    );
  }

  return { base, plugins };
}
