// Hashing primitives for cNFT merkle math. Every hash Bubblegum and
// spl-account-compression compute is keccak256 (Ethereum-style keccak,
// not SHA-3). The node-pair hash, leaf-schema hash, data hash, and
// creator hash all reduce to the same two primitives below.
//
// This file is deliberately pure and dep-minimal. Only `@noble/hashes`
// for keccak256; no Solana types, no I/O. That makes it trivial to port
// to Rust (where `solana_program::keccak::hashv` is the analogue).

import { keccak_256 } from "@noble/hashes/sha3.js";
import type { Creator, LeafSchemaV1 } from "./types.js";

/** keccak256 over a single buffer. */
export function keccak256(data: Uint8Array): Uint8Array {
  return keccak_256(data);
}

/**
 * keccak256(left || right). This is the node-pair hash used by
 * spl-account-compression's `hashv(&[left, right])`. Inputs are
 * 32 bytes each; output is 32 bytes.
 */
export function hashPair(left: Uint8Array, right: Uint8Array): Uint8Array {
  const buf = new Uint8Array(left.length + right.length);
  buf.set(left, 0);
  buf.set(right, left.length);
  return keccak_256(buf);
}

// Memoize the empty-node cascade. EMPTY[0] is the zero hash; EMPTY[i] is
// hashPair(EMPTY[i-1], EMPTY[i-1]). A tree of depth D needs EMPTY[0..D].
// Depth 30 is well above any real Bubblegum tree, so caching up front is
// trivial. We lazy-extend only when a caller asks beyond the current top.
const EMPTY_CASCADE: Uint8Array[] = [new Uint8Array(32)];

/** Hash of an all-empty subtree at the given height. `height(0)` is a leaf slot. */
export function emptyNode(height: number): Uint8Array {
  if (height < 0 || !Number.isInteger(height)) {
    throw new Error(`emptyNode: height must be a non-negative integer, got ${height}`);
  }
  while (EMPTY_CASCADE.length <= height) {
    const prev = EMPTY_CASCADE[EMPTY_CASCADE.length - 1]!;
    EMPTY_CASCADE.push(hashPair(prev, prev));
  }
  return EMPTY_CASCADE[height]!;
}

/**
 * Leaf hash per LeafSchema::V1. Bubblegum computes:
 *
 *     keccak256(
 *       0x01 || id || owner || delegate || nonce.to_le_bytes() ||
 *       data_hash || creator_hash
 *     )
 *
 * The 0x01 prefix is the schema version discriminator; V2 leaves use
 * their own format and aren't covered here.
 */
export function hashLeafV1(leaf: LeafSchemaV1): Uint8Array {
  if (leaf.id.length !== 32) throw new Error("hashLeafV1: id must be 32 bytes");
  if (leaf.owner.length !== 32) throw new Error("hashLeafV1: owner must be 32 bytes");
  if (leaf.delegate.length !== 32) throw new Error("hashLeafV1: delegate must be 32 bytes");
  if (leaf.dataHash.length !== 32) throw new Error("hashLeafV1: dataHash must be 32 bytes");
  if (leaf.creatorHash.length !== 32) throw new Error("hashLeafV1: creatorHash must be 32 bytes");

  const buf = new Uint8Array(1 + 32 + 32 + 32 + 8 + 32 + 32);
  let o = 0;
  buf[o++] = 0x01; // LeafSchema::V1 discriminator
  buf.set(leaf.id, o); o += 32;
  buf.set(leaf.owner, o); o += 32;
  buf.set(leaf.delegate, o); o += 32;
  writeU64LE(buf, o, leaf.nonce); o += 8;
  buf.set(leaf.dataHash, o); o += 32;
  buf.set(leaf.creatorHash, o);
  return keccak_256(buf);
}

/**
 * Data hash: keccak256 of the Borsh-serialized MetadataArgs. We take the
 * serialized bytes as input rather than the struct because MetadataArgs
 * serialization lives in the generated Bubblegum client — keeping this
 * module Solana-type-free.
 */
export function hashMetadataArgsBytes(metadataArgsBytes: Uint8Array): Uint8Array {
  return keccak_256(metadataArgsBytes);
}

/**
 * Creator hash: keccak256 over concatenated (address, verified, share)
 * tuples — 32 + 1 + 1 = 34 bytes per creator. Matches the Bubblegum
 * reference impl.
 */
export function hashCreators(creators: Creator[]): Uint8Array {
  const buf = new Uint8Array(creators.length * 34);
  let o = 0;
  for (const c of creators) {
    if (c.address.length !== 32) {
      throw new Error("hashCreators: address must be 32 bytes");
    }
    if (!Number.isInteger(c.share) || c.share < 0 || c.share > 255) {
      throw new Error(`hashCreators: share out of u8 range: ${c.share}`);
    }
    buf.set(c.address, o); o += 32;
    buf[o++] = c.verified ? 1 : 0;
    buf[o++] = c.share;
  }
  return keccak_256(buf);
}

function writeU64LE(buf: Uint8Array, offset: number, value: bigint): void {
  if (value < 0n || value > 0xffff_ffff_ffff_ffffn) {
    throw new Error(`writeU64LE: value out of range: ${value}`);
  }
  let v = value;
  for (let i = 0; i < 8; i++) {
    buf[offset + i] = Number(v & 0xffn);
    v >>= 8n;
  }
}
