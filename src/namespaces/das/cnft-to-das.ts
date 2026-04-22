// Map a cNFT LeafRecord to the DAS asset shape Helius returns. Pure —
// no I/O — so it's trivially testable. The only Kit dependency is the
// Address decoder for base58 encoding of the 32-byte fields we carry
// as Uint8Array internally.

import { getAddressDecoder, getBase58Decoder } from "@solana/kit";
import type { DasAsset } from "../../decoders/index.js";
import type { LeafRecord } from "../../cnft/index.js";

const addrDecoder = getAddressDecoder();
const base58 = getBase58Decoder();

function addressFromBytes(bytes: Uint8Array): string {
  return addrDecoder.decode(bytes) as string;
}

function toBase58(bytes: Uint8Array): string {
  return base58.decode(bytes);
}

export function leafRecordToDasAsset(record: LeafRecord): DasAsset {
  const m = record.mintMetadata;

  const creators = m.creators.map((c) => ({
    address: addressFromBytes(c.address),
    share: c.share,
    verified: c.verified,
  }));

  const grouping: DasAsset["grouping"] = m.collection
    ? [
        {
          group_key: "collection",
          group_value: addressFromBytes(m.collection.key),
        },
      ]
    : [];

  const owner = addressFromBytes(record.owner);
  const delegate = addressFromBytes(record.delegate);
  const delegated = owner !== delegate;

  return {
    id: record.assetId as string,
    interface: "V1_NFT",
    content: {
      $schema: "https://schema.metaplex.com/nft1.0.json",
      json_uri: m.uri,
      metadata: {
        name: m.name,
        symbol: m.symbol,
        description: "",
      },
      links: {
        image: null,
        animation_url: null,
      },
      files: [],
    },
    authorities: [],
    creators,
    ownership: {
      frozen: false,
      delegated,
      ownership_model: "single",
      owner,
    },
    grouping,
    mutable: m.isMutable,
    burnt: record.burned,
    compression: {
      eligible: true,
      compressed: true,
      data_hash: toBase58(record.dataHash),
      creator_hash: toBase58(record.creatorHash),
      asset_hash: toBase58(record.leafHash),
      tree: record.tree as string,
      seq: 0,
      leaf_id: Number(record.leafIndex),
    },
  };
}
