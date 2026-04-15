// helius.das.getNftEditions — list the print editions of a legacy
// Metaplex master NFT.
//
// Implementation:
//   - Input is the master NFT's mint address (matches Helius's schema).
//   - We derive the master edition PDA (same derivation as any Token
//     Metadata edition PDA — see pdas.ts) and decode supply /
//     max_supply from the on-chain MasterEditionV1/V2 account. That
//     part is EXACT.
//   - The `editions[]` list is LOCAL_INDEX: it reflects print editions
//     the proxy has observed via fetch.ts during a Token Metadata
//     decode. Prints that were never fetched through this proxy won't
//     appear. Fetch a print's mint first and it'll show up on the
//     next call here.
//   - Pagination is applied in-memory against the indexed list.

import type { Handler } from "../../context.js";
import { jsonRpcError, jsonRpcResult } from "../../context.js";
import { TOKEN_METADATA_PROGRAM_ID } from "../../decoders/token-metadata.js";
import { getMasterEditionV2Decoder } from "../../generated/token-metadata/accounts/masterEditionV2.js";
import { getMasterEditionV1Decoder } from "../../generated/token-metadata/accounts/masterEditionV1.js";
import { Key } from "../../generated/token-metadata/types/key.js";
import { deriveEditionPda } from "./pdas.js";

interface GetNftEditionsParams {
  mint: string;
  page?: number;
  limit?: number;
}

const masterV2Decoder = getMasterEditionV2Decoder();
const masterV1Decoder = getMasterEditionV1Decoder();

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

  const masterEditionAddress = await deriveEditionPda(mint);
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

  const allEditions = await ctx.cache.getEditionsByMaster(masterEditionAddress);
  const start = Math.max(0, (page - 1) * limit);
  const pagedEditions = allEditions.slice(start, start + limit);

  return jsonRpcResult(id, {
    total: allEditions.length,
    limit,
    page,
    master_edition_address: masterEditionAddress,
    supply: supply.supply,
    max_supply: supply.maxSupply,
    editions: pagedEditions,
  });
};
