// helius.das.getNftEditions — list the print editions of a legacy
// Metaplex master NFT.
//
// Implementation notes:
//   - Input is the master NFT's mint address (matches Helius's schema).
//   - We derive the master edition PDA (`["metadata", TM_PROGRAM, mint,
//     "edition"]`), fetch it, and decode supply / max_supply from the
//     on-chain account. That part is EXACT — pure local account read.
//   - The `editions[]` list is LOCAL_INDEX: we don't ship a background
//     indexer that reconstructs every print's mint from transaction
//     history, so for v0.5.0 we return an empty list with a `note`
//     explaining that prints surface here after they've been fetched
//     through this proxy. This matches how LOCAL_INDEX works elsewhere
//     — queries for state we haven't observed return empty.
//
// A future release can replace the empty list by hooking into
// fetch.ts's decoder pipeline and recording EditionV1 accounts as they
// flow through. That's additive: the handler shape and manifest entry
// don't change.

import {
  getAddressEncoder,
  getProgramDerivedAddress,
  type Address,
} from "@solana/kit";
import type { Handler } from "../../context.js";
import { jsonRpcError, jsonRpcResult } from "../../context.js";
import { TOKEN_METADATA_PROGRAM_ID } from "../../decoders/token-metadata.js";
import { getMasterEditionV2Decoder } from "../../generated/token-metadata/accounts/masterEditionV2.js";
import { getMasterEditionV1Decoder } from "../../generated/token-metadata/accounts/masterEditionV1.js";
import { Key } from "../../generated/token-metadata/types/key.js";

interface GetNftEditionsParams {
  mint: string;
  page?: number;
  limit?: number;
}

const addressEncoder = getAddressEncoder();
const METADATA_SEED = new TextEncoder().encode("metadata");
const EDITION_SEED = new TextEncoder().encode("edition");

const masterV2Decoder = getMasterEditionV2Decoder();
const masterV1Decoder = getMasterEditionV1Decoder();

// Master edition PDA: ["metadata", TOKEN_METADATA_PROGRAM_ID, mint, "edition"].
async function deriveMasterEditionPda(mint: string): Promise<string> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: TOKEN_METADATA_PROGRAM_ID as Address,
    seeds: [
      METADATA_SEED,
      addressEncoder.encode(TOKEN_METADATA_PROGRAM_ID as Address),
      addressEncoder.encode(mint as Address),
      EDITION_SEED,
    ],
  });
  return pda;
}

function decodeMasterSupply(
  data: Uint8Array,
): { supply: number; maxSupply: number } | null {
  if (data.length === 0) return null;
  const key = data[0];
  if (key === Key.MasterEditionV2) {
    const decoded = masterV2Decoder.decode(data);
    const max =
      decoded.maxSupply.__option === "Some"
        ? Number(decoded.maxSupply.value)
        : 0;
    return { supply: Number(decoded.supply), maxSupply: max };
  }
  if (key === Key.MasterEditionV1) {
    const decoded = masterV1Decoder.decode(data);
    const max =
      decoded.maxSupply.__option === "Some"
        ? Number(decoded.maxSupply.value)
        : 0;
    return { supply: Number(decoded.supply), maxSupply: max };
  }
  return null;
}

export const getNftEditions: Handler = async (ctx, params, id) => {
  const { mint, page = 1, limit = 100 } = (params ?? {}) as GetNftEditionsParams;
  if (!mint) {
    return jsonRpcError(id, -32602, "Missing required parameter: mint");
  }

  const masterEditionAddress = await deriveMasterEditionPda(mint);
  const account = await ctx.upstream.getAccount(masterEditionAddress);
  if (!account) {
    return jsonRpcError(
      id,
      -32000,
      `No master edition at ${masterEditionAddress} — is ${mint} a master edition mint?`,
    );
  }
  if (account.owner !== TOKEN_METADATA_PROGRAM_ID) {
    return jsonRpcError(
      id,
      -32000,
      `Account ${masterEditionAddress} is not owned by the Token Metadata program`,
    );
  }

  const supply = decodeMasterSupply(account.data);
  if (!supply) {
    return jsonRpcError(
      id,
      -32000,
      `Account ${masterEditionAddress} is not a MasterEdition`,
    );
  }

  return jsonRpcResult(id, {
    total: 0,
    limit,
    page,
    master_edition_address: masterEditionAddress,
    supply: supply.supply,
    max_supply: supply.maxSupply,
    editions: [],
    note: "surfpool-helius v0.5.0: the supply numbers come straight from the on-chain master edition account (EXACT). The editions list is LOCAL_INDEX and currently returns empty — a background indexer for discovered prints lands in a follow-up release.",
  });
};
