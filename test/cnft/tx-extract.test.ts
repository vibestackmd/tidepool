// Pure tx-shape parsing. No RPC, no Bubblegum ix decoding — just the
// wire-format → ExtractedIx mapping, with base58 data decoded.

import { test } from "node:test";
import assert from "node:assert/strict";
import { getBase58Decoder } from "@solana/kit";
import { extractBubblegumIxs } from "../../src/cnft/tx-extract.js";
import { BUBBLEGUM_PROGRAM_ADDRESS } from "../../src/cnft/parser.js";

const BG = BUBBLEGUM_PROGRAM_ADDRESS as unknown as string;
const OTHER_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TREE = "11111111111111111111111111111111";
const OWNER = "SysvarRent111111111111111111111111111111111";

// Kit's base58 "decoder" is bytes→string (decoding the wire format into
// a human-readable base58 representation). Matches what getTransaction
// returns on the ix.data field.
const base58 = getBase58Decoder();
function enc(bytes: Uint8Array): string {
  return base58.decode(bytes);
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

test("tx-extract: picks up a single outer Bubblegum ix", () => {
  const ixData = new Uint8Array([1, 2, 3, 4]);
  const tx = {
    meta: { err: null, innerInstructions: [] },
    transaction: {
      message: {
        accountKeys: [OWNER, TREE, BG],
        instructions: [
          { programIdIndex: 2, accounts: [0, 1], data: enc(ixData) },
        ],
      },
    },
  };
  const ixs = extractBubblegumIxs(tx);
  assert.equal(ixs.length, 1);
  assert.equal(hex(ixs[0]!.data), "01020304");
  assert.equal(ixs[0]!.accounts[0], OWNER);
  assert.equal(ixs[0]!.accounts[1], TREE);
});

test("tx-extract: ignores ixs that don't target Bubblegum", () => {
  const tx = {
    meta: { err: null, innerInstructions: [] },
    transaction: {
      message: {
        accountKeys: [OWNER, TREE, OTHER_PROGRAM],
        instructions: [
          { programIdIndex: 2, accounts: [0, 1], data: enc(new Uint8Array([1])) },
        ],
      },
    },
  };
  assert.equal(extractBubblegumIxs(tx).length, 0);
});

test("tx-extract: failed txs return no ixs (err !== null)", () => {
  const tx = {
    meta: { err: { InstructionError: [0, "Custom"] }, innerInstructions: [] },
    transaction: {
      message: {
        accountKeys: [OWNER, TREE, BG],
        instructions: [
          { programIdIndex: 2, accounts: [0, 1], data: enc(new Uint8Array([1])) },
        ],
      },
    },
  };
  assert.equal(extractBubblegumIxs(tx).length, 0);
});

test("tx-extract: follows inner ixs under the matching outer ix", () => {
  const outerData = new Uint8Array([0xaa]);
  const innerData = new Uint8Array([0xbb, 0xcc]);
  const tx = {
    meta: {
      err: null,
      innerInstructions: [
        {
          index: 0, // under the first outer ix
          instructions: [
            { programIdIndex: 3, accounts: [1], data: enc(innerData) },
          ],
        },
      ],
    },
    transaction: {
      message: {
        accountKeys: [OWNER, TREE, OTHER_PROGRAM, BG],
        instructions: [
          // Outer ix hits OTHER_PROGRAM (wrapper) but CPIs into Bubblegum.
          { programIdIndex: 2, accounts: [0, 1], data: enc(outerData) },
        ],
      },
    },
  };
  const ixs = extractBubblegumIxs(tx);
  assert.equal(ixs.length, 1);
  assert.equal(hex(ixs[0]!.data), "bbcc");
  assert.equal(ixs[0]!.accounts[0], TREE);
});

test("tx-extract: resolves loadedAddresses (versioned txs)", () => {
  const tx = {
    meta: {
      err: null,
      innerInstructions: [],
      loadedAddresses: {
        writable: [TREE],     // index 2
        readonly: [BG],       // index 3
      },
    },
    transaction: {
      message: {
        accountKeys: [OWNER, OTHER_PROGRAM], // 2 static keys
        instructions: [
          // programIdIndex 3 → readonly[0] = BG
          // accounts [0, 2] → [OWNER, TREE]
          { programIdIndex: 3, accounts: [0, 2], data: enc(new Uint8Array([1])) },
        ],
      },
    },
  };
  const ixs = extractBubblegumIxs(tx);
  assert.equal(ixs.length, 1);
  assert.equal(ixs[0]!.accounts[0], OWNER);
  assert.equal(ixs[0]!.accounts[1], TREE);
});

test("tx-extract: preserves outer-then-inner order within a tx", () => {
  const outer = new Uint8Array([0x01]);
  const inner = new Uint8Array([0x02]);
  const tx = {
    meta: {
      err: null,
      innerInstructions: [
        { index: 0, instructions: [{ programIdIndex: 2, accounts: [0, 1], data: enc(inner) }] },
      ],
    },
    transaction: {
      message: {
        accountKeys: [OWNER, TREE, BG],
        instructions: [
          { programIdIndex: 2, accounts: [0, 1], data: enc(outer) },
        ],
      },
    },
  };
  const ixs = extractBubblegumIxs(tx);
  assert.equal(ixs.length, 2);
  assert.equal(hex(ixs[0]!.data), "01");
  assert.equal(hex(ixs[1]!.data), "02");
});

test("tx-extract: empty or malformed tx returns []", () => {
  assert.deepEqual(extractBubblegumIxs({}), []);
  assert.deepEqual(extractBubblegumIxs({ meta: null }), []);
  assert.deepEqual(
    extractBubblegumIxs({ meta: { err: null }, transaction: null }),
    [],
  );
});

test("tx-extract: pairs outer Bubblegum ix with inner noop LeafSchemaEvent", async () => {
  // Lazy imports — only this test needs the leaf-event synth helpers.
  const { getLeafSchemaEncoder } = await import(
    "../../src/generated/bubblegum/types/leafSchema.js"
  );
  const { getAddressDecoder } = await import("@solana/kit");
  const addrDec = getAddressDecoder();
  const addrOf = (b: number) =>
    addrDec.decode(new Uint8Array(32).fill(b)) as string;

  const SPL_NOOP = "noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV";
  const assetId = addrOf(0x99);
  const owner = addrOf(0x11);
  const delegate = addrOf(0x22);
  const schemaBytes = getLeafSchemaEncoder().encode({
    __kind: "V1",
    id: assetId as unknown as Parameters<
      ReturnType<typeof getLeafSchemaEncoder>["encode"]
    >[0] extends infer X
      ? X extends { id: infer A }
        ? A
        : never
      : never,
    owner: owner as never,
    delegate: delegate as never,
    nonce: 3n,
    dataHash: new Uint8Array(32).fill(0xaa),
    creatorHash: new Uint8Array(32).fill(0xbb),
  });
  // Event wire: event_type=1, version=0, LeafSchema bytes, op byte=4.
  const leafEvent = new Uint8Array(1 + 1 + schemaBytes.length + 1);
  leafEvent[0] = 1;
  leafEvent[1] = 0;
  leafEvent.set(schemaBytes, 2);
  leafEvent[leafEvent.length - 1] = 4;

  const tx = {
    meta: {
      err: null,
      innerInstructions: [
        {
          index: 0,
          instructions: [
            // First inner ix: Bubblegum's own noop CPI with LeafSchemaEvent
            {
              programIdIndex: 3,
              accounts: [],
              data: enc(leafEvent),
            },
          ],
        },
      ],
    },
    transaction: {
      message: {
        accountKeys: [OWNER, TREE, BG, SPL_NOOP],
        instructions: [
          {
            programIdIndex: 2, // BG
            accounts: [0, 1],
            data: enc(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8])), // junk discriminator
          },
        ],
      },
    },
  };

  const ixs = extractBubblegumIxs(tx);
  assert.equal(ixs.length, 1);
  assert.ok(ixs[0]!.noopEvent, "outer ix should be paired with its noop event");
  assert.equal(ixs[0]!.noopEvent!.schema.kind, "V1");
});
