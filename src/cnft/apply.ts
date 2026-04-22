// applyEvent — the one place store state mutates in response to a
// parsed CnftEvent. Keeping this separate from the parser and the
// indexer lets us unit-test the state transition logic against a pure
// event stream, independent of ix decoding or upstream RPC calls.
//
// Async because of PDA derivation for asset IDs on mint events —
// @solana/kit's getProgramDerivedAddress is async. Everything else is
// straight-line store calls.

import type { Address } from "@solana/kit";
import { getAddressEncoder, getProgramDerivedAddress } from "@solana/kit";
import {
  hashCreators,
  hashLeafV1,
  hashMetadataArgsBytes,
} from "./hash.js";
import { BUBBLEGUM_PROGRAM_ADDRESS } from "./parser.js";
import type { CnftStore } from "./store.js";
import type { CnftEvent, LeafRecord, NoopOverride } from "./types.js";

/**
 * Apply a parsed event to the store. Throws only for genuinely corrupt
 * state (mutation of an unknown tree / leaf); malformed input is the
 * parser's job to reject.
 */
export async function applyEvent(
  store: CnftStore,
  event: CnftEvent,
): Promise<void> {
  switch (event.kind) {
    case "createTree":
      await store.putTree({
        tree: event.tree,
        depth: event.depth,
        maxBufferSize: event.maxBufferSize,
        numMinted: 0n,
      });
      return;

    case "mint": {
      const tree = await store.getTree(event.tree);
      if (!tree) {
        throw new Error(`applyEvent: mint on unknown tree ${event.tree}`);
      }
      // If the parser handed us a noop override, its nonce is
      // authoritative — use it and keep the store's numMinted counter
      // in sync. Otherwise allocate the next index ourselves. This
      // keeps replay deterministic across restarts even if the order
      // of scanned transactions differs from mint order (unlikely but
      // possible at the edges of pagination).
      let leafIndex: bigint;
      if (event.noop) {
        leafIndex = event.noop.nonce;
        // Ensure the store's counter reaches at least this + 1.
        while (tree.numMinted <= leafIndex) {
          await store.allocLeafIndex(event.tree);
          tree.numMinted = (tree.numMinted ?? 0n) + 1n;
        }
      } else {
        leafIndex = await store.allocLeafIndex(event.tree);
      }
      const nonce = leafIndex;
      const assetId = await deriveAssetId(event.tree, nonce);

      const reconstructedDataHash = hashMetadataArgsBytes(
        event.metadata.dataHashInput,
      );
      const reconstructedCreatorHash = hashCreators(event.metadata.creators);
      const owner = event.noop?.owner ?? event.owner;
      const delegate = event.noop?.delegate ?? event.delegate;
      const dataHash = event.noop?.dataHash ?? reconstructedDataHash;
      const creatorHash = event.noop?.creatorHash ?? reconstructedCreatorHash;
      const leafHash = hashLeafV1({
        id: addressBytes(assetId),
        owner,
        delegate,
        nonce,
        dataHash,
        creatorHash,
      });

      const record: LeafRecord = {
        assetId,
        tree: event.tree,
        nonce,
        leafIndex,
        mintMetadata: event.metadata,
        owner,
        delegate,
        dataHash,
        creatorHash,
        leafHash,
        burned: false,
      };
      await store.putLeaf(record);
      return;
    }

    case "transfer": {
      const existing = await requireLeaf(store, event.tree, event.leafIndex);
      // Sanity-check that the caller-asserted pre-image matches what we
      // have. If it doesn't, our state is stale — skip rather than
      // silently clobber. (Upstream indexer will re-scan.)
      if (!bytesEq(existing.dataHash, event.dataHash)) return;
      if (!bytesEq(existing.creatorHash, event.creatorHash)) return;

      const next: LeafRecord = {
        ...existing,
        owner: event.newOwner,
        delegate: event.newDelegate,
        leafHash: hashLeafV1({
          id: addressBytes(existing.assetId),
          owner: event.newOwner,
          delegate: event.newDelegate,
          nonce: existing.nonce,
          dataHash: existing.dataHash,
          creatorHash: existing.creatorHash,
        }),
      };
      await store.putLeaf(next);
      return;
    }

    case "delegate": {
      const existing = await requireLeaf(store, event.tree, event.leafIndex);
      if (!bytesEq(existing.dataHash, event.dataHash)) return;
      if (!bytesEq(existing.creatorHash, event.creatorHash)) return;

      const next: LeafRecord = {
        ...existing,
        delegate: event.newDelegate,
        leafHash: hashLeafV1({
          id: addressBytes(existing.assetId),
          owner: existing.owner,
          delegate: event.newDelegate,
          nonce: existing.nonce,
          dataHash: existing.dataHash,
          creatorHash: existing.creatorHash,
        }),
      };
      await store.putLeaf(next);
      return;
    }

    case "burn": {
      const existing = await requireLeaf(store, event.tree, event.leafIndex);
      const next: LeafRecord = {
        ...existing,
        burned: true,
        leafHash: new Uint8Array(32), // empty leaf → all-zeros node
      };
      await store.putLeaf(next);
      return;
    }

    case "verifyCreator":
    case "unverifyCreator": {
      await applyNoopLeafUpdate(store, event.tree, event.noop, (existing) => {
        // Mirror the creator's new verified state into the stored
        // mintMetadata so future DAS responses reflect the flip.
        const flip = event.kind === "verifyCreator";
        const creators = existing.mintMetadata.creators.map((c) =>
          bytesEq(c.address, event.creator) ? { ...c, verified: flip } : c,
        );
        return {
          mintMetadata: { ...existing.mintMetadata, creators },
        };
      });
      return;
    }

    case "verifyCollection":
    case "unverifyCollection":
    case "setAndVerifyCollection": {
      await applyNoopLeafUpdate(store, event.tree, event.noop, (existing) => {
        const verified =
          event.kind !== "unverifyCollection"; // set-and-verify + verify both flip to true
        return {
          mintMetadata: {
            ...existing.mintMetadata,
            collection: { key: event.collection, verified },
          },
        };
      });
      return;
    }

    case "updateMetadata": {
      await applyNoopLeafUpdate(store, event.tree, event.noop, (existing) => {
        // Partial update: a None field in the ix means "leave prior
        // value." Parser has already unwrapped the Options; empty
        // strings signal "not provided" in practice, but Bubblegum
        // allows explicit clears too. To preserve prior values when
        // the updateArgs carried None, prefer existing.mintMetadata's
        // fields for anything event.newMetadata left as default.
        const n = event.newMetadata;
        const prev = existing.mintMetadata;
        return {
          mintMetadata: {
            name: n.name || prev.name,
            symbol: n.symbol || prev.symbol,
            uri: n.uri || prev.uri,
            sellerFeeBasisPoints:
              n.sellerFeeBasisPoints !== 0 ? n.sellerFeeBasisPoints : prev.sellerFeeBasisPoints,
            primarySaleHappened: n.primarySaleHappened || prev.primarySaleHappened,
            isMutable: n.isMutable,
            creators: n.creators.length > 0 ? n.creators : prev.creators,
            collection: prev.collection, // updateMetadata doesn't change collection membership
            dataHashInput: n.dataHashInput,
          },
        };
      });
      return;
    }
  }
}

/**
 * Shared path for noop-authoritative updates: look up the leaf by the
 * event's authoritative id (via nonce-derived assetId), compute the
 * new leafHash from the override's owner/delegate/hashes, let the
 * caller patch whatever derived fields (mintMetadata, etc) depend on
 * the ix's non-hash semantics, then write it back.
 */
async function applyNoopLeafUpdate(
  store: CnftStore,
  tree: Address,
  noop: NoopOverride,
  patch: (existing: LeafRecord) => Partial<LeafRecord>,
): Promise<void> {
  const existing = await requireLeaf(store, tree, noop.leafIndex);
  const leafHash = hashLeafV1({
    id: addressBytes(existing.assetId),
    owner: noop.owner,
    delegate: noop.delegate,
    nonce: existing.nonce,
    dataHash: noop.dataHash,
    creatorHash: noop.creatorHash,
  });
  const caller = patch(existing);
  const next: LeafRecord = {
    ...existing,
    ...caller,
    owner: noop.owner,
    delegate: noop.delegate,
    dataHash: noop.dataHash,
    creatorHash: noop.creatorHash,
    leafHash,
  };
  await store.putLeaf(next);
}

async function requireLeaf(
  store: CnftStore,
  tree: Address,
  leafIndex: bigint,
): Promise<LeafRecord> {
  const existing = await store.getLeafByIndex(tree, leafIndex);
  if (!existing) {
    throw new Error(
      `applyEvent: expected leaf at (${tree}, ${leafIndex}) but none exists`,
    );
  }
  return existing;
}

/**
 * Bubblegum asset ID = PDA(["asset", tree.toBytes(), nonce.to_le_bytes()],
 * bubblegum_program_id). Matches mpl-bubblegum's `get_asset_id` helper.
 */
export async function deriveAssetId(
  tree: Address,
  nonce: bigint,
): Promise<Address> {
  const addressEncoder = getAddressEncoder();
  const [addr] = await getProgramDerivedAddress({
    programAddress: BUBBLEGUM_PROGRAM_ADDRESS,
    seeds: [
      new TextEncoder().encode("asset"),
      addressEncoder.encode(tree),
      u64LeBytes(nonce),
    ],
  });
  return addr;
}

function addressBytes(addr: Address): Uint8Array {
  return getAddressEncoder().encode(addr) as Uint8Array;
}

function u64LeBytes(v: bigint): Uint8Array {
  const buf = new Uint8Array(8);
  let x = v;
  for (let i = 0; i < 8; i++) {
    buf[i] = Number(x & 0xffn);
    x >>= 8n;
  }
  return buf;
}

function bytesEq(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}
