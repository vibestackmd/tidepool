// Token Metadata account decoder — legacy Metaplex NFT standard.
//
// Thin adapter over the Codama-generated Kit-native decoders at
// `src/generated/token-metadata/`. We import `getMetadataDecoder` from
// there — those come from the pinned IDL at `idls/token_metadata.json`
// (see `idls/token_metadata.source.json` for the upstream commit).
// Re-regenerate with `make update-idl-token-metadata`.
//
// This decoder only handles the Metadata account variant (key=4 /
// MetadataV1). The other program-owned account types (MasterEditionV2,
// EditionV1, EditionMarker, delegate records, etc.) return null from
// this decoder because they aren't DAS assets — they're metadata *about*
// assets. The `getNftEditions` handler reads MasterEditionV2 accounts
// directly via its own codec call.
//
// Legacy NFTs are identified in Helius's DAS API by their mint address,
// not the Metadata PDA that stores their data. So the returned DasAsset
// uses `metadata.mint` as `id` regardless of which pubkey was passed in
// — a caller can reach the same asset via either the mint (routed by
// fetch.ts's mint-as-id logic) or the Metadata PDA directly.

import type { Option } from "@solana/kit";
import type { AccountDecoder, DasAsset } from "./index.js";
import { getMetadataDecoder } from "../generated/token-metadata/accounts/metadata.js";
import { Key } from "../generated/token-metadata/types/key.js";
import { TokenStandard } from "../generated/token-metadata/types/tokenStandard.js";

export const TOKEN_METADATA_PROGRAM_ID =
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";

const metadataDecoder = getMetadataDecoder();

function unwrapOption<T>(opt: Option<T>): T | null {
  return opt.__option === "Some" ? opt.value : null;
}

// Metaplex Metadata pads name/symbol/uri to fixed max lengths
// (MAX_NAME_LENGTH=32, MAX_SYMBOL_LENGTH=10, MAX_URI_LENGTH=200) with
// trailing null bytes. The Borsh length prefix includes the padding,
// so the decoded strings arrive with \0 characters we need to strip.
function trimNulls(s: string): string {
  return s.replace(/\0+$/, "");
}

function interfaceForTokenStandard(ts: TokenStandard | null): string {
  switch (ts) {
    case TokenStandard.ProgrammableNonFungible:
    case TokenStandard.ProgrammableNonFungibleEdition:
      return "ProgrammableNFT";
    case TokenStandard.FungibleAsset:
      return "FungibleAsset";
    case TokenStandard.Fungible:
      return "FungibleToken";
    case TokenStandard.NonFungible:
    case TokenStandard.NonFungibleEdition:
    default:
      return "V1_NFT";
  }
}

/** Fetch off-chain JSON metadata. Supports http(s) and file:// URIs. */
async function fetchOffChainMetadata(
  uri: string,
): Promise<Record<string, unknown> | null> {
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

export const tokenMetadataDecoder: AccountDecoder = {
  programId: TOKEN_METADATA_PROGRAM_ID,
  name: "token-metadata",

  async decode(pubkey, data) {
    if (data.length === 0) return null;
    if (data[0] !== Key.MetadataV1) return null;

    try {
      const decoded = metadataDecoder.decode(data);

      const name = trimNulls(decoded.data.name);
      const symbol = trimNulls(decoded.data.symbol);
      const uri = trimNulls(decoded.data.uri);
      const tokenStandard = unwrapOption(decoded.tokenStandard);
      const iface = interfaceForTokenStandard(tokenStandard);
      const mint = decoded.mint as string;
      const rawCreators = unwrapOption(decoded.data.creators) ?? [];
      const collection = unwrapOption(decoded.collection);

      const offChain = await fetchOffChainMetadata(uri);
      const props = (offChain?.properties ?? {}) as Record<string, unknown>;
      const files = (props.files ?? []) as Array<{ uri?: string; type?: string }>;

      const creators = rawCreators.map((c) => ({
        address: c.address as string,
        share: c.share,
        verified: c.verified,
      }));

      // Only "verified" collections flow into DAS grouping — an
      // unverified collection pointer is just an unauthenticated claim.
      const grouping =
        collection && collection.verified
          ? [{ group_key: "collection", group_value: collection.key as string }]
          : [];

      return {
        id: mint,
        interface: iface,
        content: {
          $schema: "https://schema.metaplex.com/nft/1.0",
          json_uri: uri,
          metadata: {
            name,
            symbol,
            description: (offChain?.description as string) ?? "",
          },
          links: {
            image: (offChain?.image as string) ?? null,
            animation_url: (offChain?.animation_url as string) ?? null,
          },
          files: files.map((f) => ({ uri: f.uri ?? "", mime: f.type ?? "" })),
        },
        authorities: [
          { address: decoded.updateAuthority as string, scopes: ["full"] },
        ],
        creators,
        ownership: {
          frozen: false,
          delegated: false,
          ownership_model: "single",
          // Owner lives on the mint's token account, not the Metadata
          // PDA. fetch.ts resolves this via a getProgramAccounts scan
          // of SPL Token accounts filtered by the mint, after the
          // decoder returns; we leave it empty here so the decoder
          // stays a pure account→DAS transform.
          owner: "",
        },
        grouping,
        mutable: decoded.isMutable,
        burnt: false,
      } satisfies DasAsset;
    } catch (err) {
      console.error(
        `[token-metadata] Failed to decode ${pubkey}: ${(err as Error).message}`,
      );
      return null;
    }
  },
};
