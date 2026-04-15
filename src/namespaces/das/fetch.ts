// Shared DAS fetch helper. `getAsset` and any future method that needs to
// materialize an asset from an on-chain address goes through this — hit
// the cache first, then fall back to the upstream account read + decoder
// pipeline, then populate the cache on success.
//
// Three non-trivial routing concerns live here and not in the decoders:
//
//   1. Mint-as-id routing. Real Helius's DAS API accepts a legacy NFT's
//      *mint* as the asset id, but the metadata is stored at a PDA
//      derived from that mint. If the address the caller passed is an
//      SPL Token or Token-2022 mint, we derive the Metaplex Metadata
//      PDA and re-fetch before handing off to the decoder.
//
//   2. Owner resolution. A Metaplex Metadata account doesn't store the
//      NFT's current owner — the owner lives on the mint's token
//      account. The token-metadata decoder leaves `ownership.owner`
//      empty on purpose, and we look it up here via a
//      getProgramAccounts memcmp scan of the token program that owns
//      the mint (Surfpool's `getTokenLargestAccounts` implementation
//      times out, so the memcmp approach is what we actually ship).
//
//   3. Print edition indexing. After a legacy-NFT decode we also fetch
//      the mint's Edition PDA. If it holds an `EditionV1` (key=1) the
//      mint is a print and we record it in the cache under its parent
//      master edition PDA, so subsequent `getNftEditions` calls on the
//      master can return the print. If the account holds a master
//      edition we skip — the master itself is the query key, not an
//      indexed row. This is the side effect that makes LOCAL_INDEX
//      semantics work for the editions[] list.

import { getAddressDecoder } from "@solana/kit";
import type { DasAsset } from "../../decoders/index.js";
import type { RequestContext } from "../../context.js";
import { TOKEN_METADATA_PROGRAM_ID } from "../../decoders/token-metadata.js";
import { getEditionDecoder } from "../../generated/token-metadata/accounts/edition.js";
import { Key } from "../../generated/token-metadata/types/key.js";
import { deriveEditionPda, deriveMetadataPda } from "./pdas.js";

// SPL Token is the classic 82-byte-mint program. Token-2022 mints can
// carry TLV extensions past byte 82, so we accept any length ≥ 82 for
// either program. Both lay out the mint field the same way at offset 0
// of a token account, and both Metaplex Metadata PDAs derive
// identically — the only thing that changes is which program's
// accounts we scan when resolving the holder.
const SPL_TOKEN_PROGRAM_ID = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const SPL_TOKEN_MIN_MINT_SIZE = 82;
const TOKEN_PROGRAMS = new Set([SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID]);

const addressDecoder = getAddressDecoder();
const editionDecoder = getEditionDecoder();

interface RawProgramAccount {
  pubkey: string;
  account: {
    data: [string, string];
    owner: string;
    lamports: number;
  };
}

// Scan the given token program for accounts whose first 32 bytes
// (token account `mint` field) match the queried mint. memcmp at
// offset 0 is specific enough — mint accounts put `mint_authority` at
// a different offset, so false positives are impossible. For NFTs
// there's exactly one non-zero holder; for fungibles we pick the
// largest-balance holder. The dataSize filter is intentionally dropped
// so Token-2022 accounts with extensions still match.
async function resolveMintHolder(
  ctx: RequestContext,
  tokenProgramId: string,
  mint: string,
): Promise<string | null> {
  try {
    const result = (await ctx.upstream.rpcCall("getProgramAccounts", [
      tokenProgramId,
      {
        encoding: "base64",
        filters: [{ memcmp: { offset: 0, bytes: mint } }],
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

// If the mint's Edition PDA holds an EditionV1 account (print edition)
// we record it in the cache under its parent master edition PDA.
// Master editions (key=MasterEditionV1/V2) are not indexed — their
// address is the query key for getNftEditions, not an indexed row.
// All errors here are swallowed because this is an optional side
// effect; the main getAsset response is already built by the caller.
async function maybeIndexPrintEdition(
  ctx: RequestContext,
  mint: string,
): Promise<void> {
  try {
    const editionPda = await deriveEditionPda(mint);
    const account = await ctx.upstream.getAccount(editionPda);
    if (!account || account.owner !== TOKEN_METADATA_PROGRAM_ID) return;
    if (account.data.length === 0 || account.data[0] !== Key.EditionV1) return;
    const decoded = editionDecoder.decode(account.data);
    await ctx.cache.putEdition(decoded.parent as string, {
      mint,
      edition_address: editionPda,
      edition: Number(decoded.edition),
    });
  } catch {
    // ignore — indexing is best-effort
  }
}

export async function fetchAndCacheAsset(
  ctx: RequestContext,
  address: string,
): Promise<DasAsset | null> {
  let account = await ctx.upstream.getAccount(address);
  if (!account) return null;

  // Mint-as-id routing. Remember which token program owned the mint so
  // owner resolution can scan the right program's accounts.
  let mintTokenProgram: string | null = null;
  if (
    TOKEN_PROGRAMS.has(account.owner) &&
    account.data.length >= SPL_TOKEN_MIN_MINT_SIZE
  ) {
    mintTokenProgram = account.owner;
    const metadataPda = await deriveMetadataPda(address);
    const metadataAccount = await ctx.upstream.getAccount(metadataPda);
    if (!metadataAccount) return null;
    account = metadataAccount;
  }

  const decoder = ctx.decoders.find((d) => d.programId === account.owner);
  if (!decoder) return null;

  const asset = await decoder.decode(address, account.data);
  if (!asset) return null;

  if (decoder.programId === TOKEN_METADATA_PROGRAM_ID) {
    if (!asset.ownership.owner) {
      const owner = await resolveMintHolder(
        ctx,
        mintTokenProgram ?? SPL_TOKEN_PROGRAM_ID,
        asset.id,
      );
      if (owner) asset.ownership.owner = owner;
    }
    // Best-effort side effect: record this mint in the edition index
    // if it turns out to be a print. Runs even when the user called
    // getAsset directly on a Metadata PDA — we still know the mint
    // from the decoded struct, and the Edition PDA derives from it.
    await maybeIndexPrintEdition(ctx, asset.id);
  }

  await ctx.cache.putAsset(asset);
  return asset;
}
