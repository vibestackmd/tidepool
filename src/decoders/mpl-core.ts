/**
 * MplCore account decoder.
 *
 * Parses AssetV1 and CollectionV1 accounts from raw bytes. Implements the minimum
 * Borsh deserialization needed to populate a DAS asset — name, uri, owner,
 * update authority, and collection grouping. Plugin data is intentionally skipped.
 *
 * Why hand-rolled instead of depending on `@metaplex-foundation/mpl-core`?
 *   - That package is UMI-flavored and pulls in a large dependency tree that
 *     doesn't fit a lean proxy.
 *   - The fields we need are a tiny prefix of each account, so hand-parsing is
 *     small, fast, and transparent.
 *   - If MplCore ever changes its account layout, swap in a Codama-generated
 *     decoder behind the same `AccountDecoder` interface.
 *
 * Reference layout (mpl-core Rust source):
 *
 *   pub struct AssetV1 {
 *     pub key: Key,                       // u8
 *     pub owner: Pubkey,                  // 32 bytes
 *     pub update_authority: UpdateAuthority, // tagged enum
 *     pub name: String,                   // u32 LE length + bytes
 *     pub uri: String,                    // u32 LE length + bytes
 *     // ...plugins (skipped)
 *   }
 *
 *   pub enum UpdateAuthority {
 *     None,                               // tag 0
 *     Address(Pubkey),                    // tag 1 + 32 bytes
 *     Collection(Pubkey),                 // tag 2 + 32 bytes
 *   }
 *
 *   pub struct CollectionV1 {
 *     pub key: Key,                       // u8
 *     pub update_authority: Pubkey,       // 32 bytes
 *     pub name: String,
 *     pub uri: String,
 *     // ...plugins (skipped)
 *   }
 */

import bs58 from "bs58";
import type { AccountDecoder, DasAsset } from "./index.js";

export const MPL_CORE_PROGRAM_ID = "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d";

const KEY_ASSET_V1 = 1;
const KEY_COLLECTION_V1 = 5;

const UA_NONE = 0;
const UA_ADDRESS = 1;
const UA_COLLECTION = 2;

class ByteReader {
  private offset = 0;

  constructor(private readonly data: Uint8Array) {}

  readU8(): number {
    if (this.offset >= this.data.length) throw new Error("readU8: out of bounds");
    return this.data[this.offset++];
  }

  readU32LE(): number {
    if (this.offset + 4 > this.data.length) throw new Error("readU32LE: out of bounds");
    const v =
      this.data[this.offset] |
      (this.data[this.offset + 1] << 8) |
      (this.data[this.offset + 2] << 16) |
      (this.data[this.offset + 3] << 24);
    this.offset += 4;
    return v >>> 0;
  }

  readPubkey(): string {
    if (this.offset + 32 > this.data.length) throw new Error("readPubkey: out of bounds");
    const bytes = this.data.slice(this.offset, this.offset + 32);
    this.offset += 32;
    return bs58.encode(bytes);
  }

  readString(): string {
    const len = this.readU32LE();
    if (this.offset + len > this.data.length) throw new Error("readString: out of bounds");
    const bytes = this.data.slice(this.offset, this.offset + len);
    this.offset += len;
    return new TextDecoder("utf-8").decode(bytes);
  }
}

interface ParsedAsset {
  owner: string;
  updateAuthority: { kind: "None" } | { kind: "Address"; pubkey: string } | { kind: "Collection"; pubkey: string };
  name: string;
  uri: string;
}

function parseAssetV1(data: Uint8Array): ParsedAsset {
  const r = new ByteReader(data);
  r.readU8(); // key discriminator
  const owner = r.readPubkey();

  const uaTag = r.readU8();
  let updateAuthority: ParsedAsset["updateAuthority"];
  if (uaTag === UA_NONE) {
    updateAuthority = { kind: "None" };
  } else if (uaTag === UA_ADDRESS) {
    updateAuthority = { kind: "Address", pubkey: r.readPubkey() };
  } else if (uaTag === UA_COLLECTION) {
    updateAuthority = { kind: "Collection", pubkey: r.readPubkey() };
  } else {
    throw new Error(`Unknown UpdateAuthority tag: ${uaTag}`);
  }

  const name = r.readString();
  const uri = r.readString();

  return { owner, updateAuthority, name, uri };
}

interface ParsedCollection {
  updateAuthority: string;
  name: string;
  uri: string;
}

function parseCollectionV1(data: Uint8Array): ParsedCollection {
  const r = new ByteReader(data);
  r.readU8(); // key discriminator
  const updateAuthority = r.readPubkey();
  const name = r.readString();
  const uri = r.readString();
  return { updateAuthority, name, uri };
}

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

function buildDasAsset(
  id: string,
  iface: "MplCoreAsset" | "MplCoreCollection",
  name: string,
  uri: string,
  owner: string,
  grouping: Array<{ group_key: string; group_value: string }>,
  metadata: Record<string, unknown> | null
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

export const mplCoreDecoder: AccountDecoder = {
  programId: MPL_CORE_PROGRAM_ID,
  name: "mpl-core",

  async decode(pubkey, data) {
    if (data.length === 0) return null;
    const key = data[0];

    try {
      if (key === KEY_ASSET_V1) {
        const parsed = parseAssetV1(data);
        const metadata = await fetchMetadata(parsed.uri);

        const grouping =
          parsed.updateAuthority.kind === "Collection"
            ? [{ group_key: "collection", group_value: parsed.updateAuthority.pubkey }]
            : [];

        return buildDasAsset(
          pubkey,
          "MplCoreAsset",
          parsed.name,
          parsed.uri,
          parsed.owner,
          grouping,
          metadata
        );
      }

      if (key === KEY_COLLECTION_V1) {
        const parsed = parseCollectionV1(data);
        const metadata = await fetchMetadata(parsed.uri);

        return buildDasAsset(
          pubkey,
          "MplCoreCollection",
          parsed.name,
          parsed.uri,
          parsed.updateAuthority,
          [],
          metadata
        );
      }
    } catch (err) {
      console.error(`[mpl-core] Failed to decode ${pubkey}: ${(err as Error).message}`);
    }

    return null;
  },
};
