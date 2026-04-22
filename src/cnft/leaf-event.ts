// LeafSchemaEvent decoder — the authoritative new-state announcement
// Bubblegum emits as an inner CPI to the noop program on every
// leaf-mutating instruction. Parsing this lets us track verifyCreator
// / verifyCollection / updateMetadata and similar ixs whose new state
// can't be reconstructed from the outer ix args alone.
//
// Wire layout (borsh-packed, per mpl-bubblegum's LeafSchemaEvent):
//
//     [0]    u8 event_type          — BubblegumEventType (1 = LeafSchemaEvent)
//     [1]    u8 version             — historical; ignored on our side
//     [2..]  LeafSchema             — V1 (0x00 disc) or V2 (0x01 disc)
//     [..]   InstructionName        — u8 enum identifying the emitting ix
//
// The same noop program (SPL Noop) is also the sink for
// spl-account-compression's ChangeLogEvent, which has a different
// layout — we distinguish by the first byte (event_type==1) cheaply
// before attempting a full decode.

import {
  getAddressEncoder,
  type Address,
} from "@solana/kit";
import { getBubblegumEventTypeDecoder } from "../generated/bubblegum/types/bubblegumEventType.js";
import { getInstructionNameDecoder } from "../generated/bubblegum/types/instructionName.js";
import {
  getLeafSchemaDecoder,
  getLeafSchemaEncoder,
} from "../generated/bubblegum/types/leafSchema.js";

// Program IDs for the two noops Bubblegum has emitted events through.
// SPL Noop is the historical + current default; MPL Noop is used by the
// V2 ix family introduced in recent Bubblegum versions. We accept both.
export const SPL_NOOP_PROGRAM_ID =
  "noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV" as Address;
export const MPL_NOOP_PROGRAM_ID =
  "mnoopTCrg4p8ry25e4bcWA9XZjbNjMTfgYVGGEdRsf3" as Address;
export const NOOP_PROGRAM_IDS: ReadonlySet<string> = new Set([
  SPL_NOOP_PROGRAM_ID as unknown as string,
  MPL_NOOP_PROGRAM_ID as unknown as string,
]);

/**
 * What we hand back on a successful decode. All 32-byte values are
 * passed through as Uint8Array so downstream logic doesn't have to
 * re-decode base58 to compare bytes.
 */
export interface LeafSchemaEventDecoded {
  /** Which on-chain instruction produced this event. Informational — applied code doesn't branch on it. */
  op: string;
  /** The authoritative new leaf schema. */
  schema:
    | {
        kind: "V1";
        id: Uint8Array;
        owner: Uint8Array;
        delegate: Uint8Array;
        nonce: bigint;
        dataHash: Uint8Array;
        creatorHash: Uint8Array;
      }
    | {
        kind: "V2";
        id: Uint8Array;
        owner: Uint8Array;
        delegate: Uint8Array;
        nonce: bigint;
        dataHash: Uint8Array;
        creatorHash: Uint8Array;
      };
}

const addrEncoder = getAddressEncoder();

function addressToBytes(addr: Address): Uint8Array {
  return addrEncoder.encode(addr) as Uint8Array;
}

/**
 * Decode LeafSchemaEvent bytes. Returns null for:
 *   - Non-LeafSchemaEvent noop payloads (wrong first byte)
 *   - Truncated / malformed bytes
 *
 * Never throws — callers treat null as "this inner ix isn't our event,
 * skip it." This is hot: every tx has multiple noop CPIs and we
 * attempt-decode each one.
 */
export function decodeLeafSchemaEvent(
  data: Uint8Array,
): LeafSchemaEventDecoded | null {
  // Cheap discriminator check before spending any decoder cycles. The
  // first byte is BubblegumEventType — 1 = LeafSchemaEvent, 0 =
  // Uninitialized. ChangeLogEvent from spl-account-compression has its
  // own layout that won't start with a `1` in the event-type slot.
  if (data.length < 3) return null;
  if (data[0] !== 1) return null;

  try {
    const eventType = getBubblegumEventTypeDecoder().decode(data.subarray(0, 1));
    if (eventType !== 1) return null;

    // Byte 1 is the historical `version` — read and discard.
    let offset = 2;

    const leafDec = getLeafSchemaDecoder();
    const schema = leafDec.decode(data.subarray(offset));

    // LeafSchema is a discriminated union, so its encoded size varies
    // between V1 and V2. Re-encode the decoded value to find how many
    // bytes it consumed, then advance past it to read the trailing
    // InstructionName. Kit doesn't expose "bytes read" on a decoder
    // directly, so this round-trip is the simplest correct approach.
    const consumed = getLeafSchemaEncoder().encode(schema).length;
    offset += consumed;

    if (offset >= data.length) return null;
    const opRaw = getInstructionNameDecoder().decode(
      data.subarray(offset, offset + 1),
    );
    const opName = typeof opRaw === "number" ? `Unknown(${opRaw})` : String(opRaw);

    return {
      op: opName,
      schema:
        schema.__kind === "V1"
          ? {
              kind: "V1",
              id: addressToBytes(schema.id),
              owner: addressToBytes(schema.owner),
              delegate: addressToBytes(schema.delegate),
              nonce: schema.nonce,
              dataHash: new Uint8Array(schema.dataHash),
              creatorHash: new Uint8Array(schema.creatorHash),
            }
          : {
              kind: "V2",
              id: addressToBytes(schema.id),
              owner: addressToBytes(schema.owner),
              delegate: addressToBytes(schema.delegate),
              nonce: schema.nonce,
              dataHash: new Uint8Array(schema.dataHash),
              creatorHash: new Uint8Array(schema.creatorHash),
            },
    };
  } catch {
    return null;
  }
}
