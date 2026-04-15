// Shared Metaplex Token Metadata PDA derivations.
//
// Two PDAs matter for DAS:
//   - Metadata PDA: ["metadata", TOKEN_METADATA_PROGRAM_ID, mint]
//       Holds the `Metadata` struct (name, symbol, uri, creators,
//       collection, update authority). Derived from the NFT's mint.
//   - Edition PDA: ["metadata", TOKEN_METADATA_PROGRAM_ID, mint, "edition"]
//       For a master edition mint this holds MasterEditionV1 or
//       MasterEditionV2; for a print edition mint it holds EditionV1.
//       Same derivation regardless of which role the mint plays —
//       dispatch happens on the account's key discriminator, not its
//       PDA path.
//
// Both fetch.ts (mint-as-id routing + edition indexing side effect)
// and the get-nft-editions handler need these. Putting them in one
// file avoids a third copy.

import {
  getAddressEncoder,
  getProgramDerivedAddress,
  type Address,
} from "@solana/kit";
import { TOKEN_METADATA_PROGRAM_ID } from "../../decoders/token-metadata.js";

const addressEncoder = getAddressEncoder();
const METADATA_SEED = new TextEncoder().encode("metadata");
const EDITION_SEED = new TextEncoder().encode("edition");

export async function deriveMetadataPda(mint: string): Promise<string> {
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

export async function deriveEditionPda(mint: string): Promise<string> {
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
