// Shared DAS fetch helper. `getAsset` and any future method that needs to
// materialize an asset from an on-chain address goes through this — hit
// the cache first, then fall back to the upstream account read + decoder
// pipeline, then populate the cache on success.
//
// Two non-trivial routing concerns live here and not in the decoders:
//
//   1. Mint-as-id routing. Real Helius's DAS API accepts a legacy NFT's
//      *mint* as the asset id, but the metadata is stored at a PDA
//      derived from that mint. If the address the caller passed is an
//      SPL Token mint, we derive the Metaplex Metadata PDA and re-fetch
//      before handing off to the decoder.
//
//   2. Owner resolution. A Metaplex Metadata account doesn't store the
//      NFT's current owner — the owner lives on the mint's token
//      account. The token-metadata decoder leaves `ownership.owner`
//      empty on purpose, and we look it up here via a
//      getProgramAccounts scan of SPL Token accounts filtered by mint
//      (Surfpool's `getTokenLargestAccounts` implementation times out,
//      so the memcmp approach is what we actually ship).
//
// Both of these only affect the Token Metadata path; MplCore assets
// short-circuit through the decoder directly as before.

import {
  getAddressDecoder,
  getAddressEncoder,
  getProgramDerivedAddress,
  type Address,
} from "@solana/kit";
import type { DasAsset } from "../../decoders/index.js";
import type { RequestContext } from "../../context.js";
import { TOKEN_METADATA_PROGRAM_ID } from "../../decoders/token-metadata.js";

// Classic SPL Token program. Token-2022 mints exist too, but their
// extension layouts are out of scope for v0.5.0 — we only route the
// classic 82-byte mint shape.
const SPL_TOKEN_PROGRAM_ID = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const SPL_TOKEN_MINT_SIZE = 82;

const addressEncoder = getAddressEncoder();
const addressDecoder = getAddressDecoder();
const METADATA_SEED = new TextEncoder().encode("metadata");

// Metaplex Metadata PDA: ["metadata", TOKEN_METADATA_PROGRAM_ID, mint].
async function deriveMetadataPda(mint: string): Promise<string> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: TOKEN_METADATA_PROGRAM_ID as Address,
    seeds: [
      METADATA_SEED,
      addressEncoder.encode(TOKEN_METADATA_PROGRAM_ID as Address),
      addressEncoder.encode(mint as Address),
    ],
  });
  return pda;
}

// SPL Token account layout: mint[0..32] owner[32..64] amount[64..72] ...
// Fixed 165 bytes total. The amount is a little-endian u64.
const SPL_TOKEN_ACCOUNT_SIZE = 165;

interface RawProgramAccount {
  pubkey: string;
  account: {
    data: [string, string];
    owner: string;
    lamports: number;
  };
}

// Scan SPL Token accounts for ones holding this mint. `getProgramAccounts`
// with a dataSize + memcmp filter is the canonical way to find holders
// of a mint, and it's what Surfpool reliably supports — its
// `getTokenLargestAccounts` times out. For NFTs (supply=1) the result
// has exactly one non-zero entry; for fungibles we pick the largest.
async function resolveMintHolder(
  ctx: RequestContext,
  mint: string,
): Promise<string | null> {
  try {
    const result = (await ctx.upstream.rpcCall("getProgramAccounts", [
      SPL_TOKEN_PROGRAM_ID,
      {
        encoding: "base64",
        filters: [
          { dataSize: SPL_TOKEN_ACCOUNT_SIZE },
          { memcmp: { offset: 0, bytes: mint } },
        ],
      },
    ])) as RawProgramAccount[] | null;
    if (!result || result.length === 0) return null;

    let bestOwner: string | null = null;
    let bestAmount = 0n;
    for (const entry of result) {
      const data = Buffer.from(entry.account.data[0], "base64");
      if (data.length < 72) continue;
      const amount = data.readBigUInt64LE(64);
      if (amount === 0n) continue;
      if (bestOwner === null || amount > bestAmount) {
        bestOwner = addressDecoder.decode(data.subarray(32, 64)) as string;
        bestAmount = amount;
      }
    }
    return bestOwner;
  } catch {
    return null;
  }
}

export async function fetchAndCacheAsset(
  ctx: RequestContext,
  address: string,
): Promise<DasAsset | null> {
  let account = await ctx.upstream.getAccount(address);
  if (!account) return null;

  // Mint-as-id routing.
  if (
    account.owner === SPL_TOKEN_PROGRAM_ID &&
    account.data.length === SPL_TOKEN_MINT_SIZE
  ) {
    const metadataPda = await deriveMetadataPda(address);
    const metadataAccount = await ctx.upstream.getAccount(metadataPda);
    if (!metadataAccount) return null;
    account = metadataAccount;
  }

  const decoder = ctx.decoders.find((d) => d.programId === account.owner);
  if (!decoder) return null;

  const asset = await decoder.decode(address, account.data);
  if (!asset) return null;

  if (!asset.ownership.owner && decoder.programId === TOKEN_METADATA_PROGRAM_ID) {
    const owner = await resolveMintHolder(ctx, asset.id);
    if (owner) asset.ownership.owner = owner;
  }

  await ctx.cache.putAsset(asset);
  return asset;
}
