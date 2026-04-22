// Pure extraction layer: given the JSON shape returned by
// `getTransaction(..., { encoding: "json" })`, walk the outer + inner
// instructions and emit every Bubblegum ix paired with its noop
// LeafSchemaEvent (when present).
//
// This is the one place that understands Solana's tx-wire shape. Kept
// separate from the async orchestrator so the mapping is easy to unit
// test with canned fixtures — no RPC mocking required.
//
// Rust-portability notes:
//   - All shapes below are plain data; nothing is coerced through a
//     class or service locator.
//   - The tx is typed loosely as `JsonRpcTransactionResponse` rather
//     than nominally because Solana RPC 1.x and RPC 2.0 differ on some
//     optional fields. We read only what we need, defensively.

import type { Address } from "@solana/kit";
import {
  decodeLeafSchemaEvent,
  NOOP_PROGRAM_IDS,
  type LeafSchemaEventDecoded,
} from "./leaf-event.js";
import { BUBBLEGUM_PROGRAM_ADDRESS } from "./parser.js";

export interface ExtractedIx {
  data: Uint8Array;
  accounts: Address[];
  /**
   * First LeafSchemaEvent emitted under this outer ix, if any.
   * Bubblegum fires it via an inner CPI to the noop program; we
   * pre-decode it here so downstream parsing doesn't need to know
   * where to look.
   */
  noopEvent?: LeafSchemaEventDecoded;
}

export interface JsonRpcInstruction {
  programIdIndex: number;
  accounts: number[];
  data: string; // base58
}

export interface JsonRpcInnerIxGroup {
  index: number;
  instructions: JsonRpcInstruction[];
}

export interface JsonRpcMeta {
  err: unknown;
  innerInstructions?: JsonRpcInnerIxGroup[] | null;
  loadedAddresses?: {
    writable?: string[];
    readonly?: string[];
  } | null;
}

export interface JsonRpcMessage {
  accountKeys: string[];
  instructions: JsonRpcInstruction[];
}

export interface JsonRpcTransactionResponse {
  meta?: JsonRpcMeta | null;
  transaction?: {
    message: JsonRpcMessage;
  } | null;
}

/**
 * Extract every Bubblegum ix from a `getTransaction` response, outer +
 * inner, preserving their submission order so state transitions replay
 * correctly. Txs that failed on-chain are skipped entirely (err !== null).
 *
 * For each outer Bubblegum ix we walk the inner-ix group at its index,
 * find the first noop CPI whose data decodes to a LeafSchemaEvent, and
 * attach it to the outer ix. For inner Bubblegum ixs (Bubblegum CPI'd
 * from a wrapper program) we look for a noop CPI that appears later in
 * the same inner group — a rough heuristic but sufficient for the
 * common wrapper case. Ixs without a paired event just omit the field.
 */
export function extractBubblegumIxs(tx: JsonRpcTransactionResponse): ExtractedIx[] {
  const meta = tx.meta;
  if (!meta || meta.err !== null) return [];
  const message = tx.transaction?.message;
  if (!message) return [];

  // Resolve the full keytable: static keys, then loaded-writable, then
  // loaded-readonly. programIdIndex + accounts[] indices target this
  // combined list. Versioned txs (v0) use the loaded-addresses
  // extensions; legacy txs have them absent.
  const keys: string[] = [
    ...message.accountKeys,
    ...(meta.loadedAddresses?.writable ?? []),
    ...(meta.loadedAddresses?.readonly ?? []),
  ];

  const out: ExtractedIx[] = [];

  const outerIxs = message.instructions ?? [];
  for (let i = 0; i < outerIxs.length; i++) {
    const ix = outerIxs[i]!;
    const innerGroup = (meta.innerInstructions ?? []).find((g) => g.index === i);

    if (isBubblegum(keys, ix.programIdIndex)) {
      const extracted = toExtracted(keys, ix);
      if (extracted) {
        extracted.noopEvent = findFirstLeafEvent(keys, innerGroup?.instructions ?? [], 0);
        out.push(extracted);
      }
    }

    if (innerGroup) {
      for (let j = 0; j < innerGroup.instructions.length; j++) {
        const inner = innerGroup.instructions[j]!;
        if (isBubblegum(keys, inner.programIdIndex)) {
          const extracted = toExtracted(keys, inner);
          if (extracted) {
            // Bubblegum's noop CPI is emitted later in the same
            // inner-ix list. Scan from the ix after this one forward.
            extracted.noopEvent = findFirstLeafEvent(
              keys,
              innerGroup.instructions,
              j + 1,
            );
            out.push(extracted);
          }
        }
      }
    }
  }

  return out;
}

function isBubblegum(keys: string[], programIdIndex: number): boolean {
  const key = keys[programIdIndex];
  return key === (BUBBLEGUM_PROGRAM_ADDRESS as unknown as string);
}

function findFirstLeafEvent(
  keys: string[],
  instructions: JsonRpcInstruction[],
  fromIndex: number,
): LeafSchemaEventDecoded | undefined {
  for (let i = fromIndex; i < instructions.length; i++) {
    const ix = instructions[i]!;
    const key = keys[ix.programIdIndex];
    if (!key || !NOOP_PROGRAM_IDS.has(key)) continue;
    const bytes = base58Decode(ix.data);
    const event = decodeLeafSchemaEvent(bytes);
    if (event) return event;
  }
  return undefined;
}

function toExtracted(keys: string[], ix: JsonRpcInstruction): ExtractedIx | null {
  const accounts: Address[] = [];
  for (const i of ix.accounts) {
    const key = keys[i];
    if (!key) return null;
    accounts.push(key as Address);
  }
  const data = base58Decode(ix.data);
  return { data, accounts };
}

// Local base58 decoder, same implementation as parser.ts. Duplicated
// rather than shared so tx-extract has no internal dep on parser.ts —
// parser.ts is the ix-semantics layer; this file is the wire-shape
// layer. Crossing the cut would make the module graph harder to
// reason about in Rust where these would be distinct modules.
const BS58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BS58_MAP: Record<string, number> = (() => {
  const m: Record<string, number> = {};
  for (let i = 0; i < BS58_ALPHABET.length; i++) m[BS58_ALPHABET[i]!] = i;
  return m;
})();

function base58Decode(s: string): Uint8Array {
  if (s.length === 0) return new Uint8Array(0);
  const bytes: number[] = [0];
  for (const ch of s) {
    const v = BS58_MAP[ch];
    if (v === undefined) throw new Error(`base58: invalid char '${ch}'`);
    let carry = v;
    for (let i = 0; i < bytes.length; i++) {
      carry += bytes[i]! * 58;
      bytes[i] = carry & 0xff;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }
  for (let i = 0; i < s.length && s[i] === "1"; i++) bytes.push(0);
  return new Uint8Array(bytes.reverse());
}
