// Direct exercise of the transport-agnostic primitives. No third-party
// library — this is the ground truth the library-specific tests depend on.
// If something breaks here, every integration breaks with it.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  createHeliusContext,
  createFixtureUpstream,
  handleJsonRpcBody,
} from "../../src/index.js";

test("handleJsonRpcBody returns a response for a known method (string body)", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const body = JSON.stringify({
    jsonrpc: "2.0",
    id: 1,
    method: "surfpoolHeliusInfo",
    params: [],
  });

  const resp = await handleJsonRpcBody(ctx, body);

  assert.ok(resp, "expected a response");
  assert.equal(resp!.jsonrpc, "2.0");
  assert.equal(resp!.id, 1);
  assert.ok("result" in resp!);
  assert.equal((resp as { result: { name: string } }).result.name, "surfpool-helius");
});

test("handleJsonRpcBody accepts a Uint8Array body", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const body = new TextEncoder().encode(
    JSON.stringify({ jsonrpc: "2.0", id: 2, method: "surfpoolHeliusInfo", params: [] }),
  );

  const resp = await handleJsonRpcBody(ctx, body);

  assert.ok(resp);
  assert.equal(resp!.id, 2);
});

test("handleJsonRpcBody accepts an already-parsed object", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });

  const resp = await handleJsonRpcBody(ctx, {
    method: "surfpoolHeliusInfo",
    id: 3,
    params: [],
  });

  assert.ok(resp);
  assert.equal(resp!.id, 3);
});

test("handleJsonRpcBody returns null for unknown methods (passthrough signal)", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const body = JSON.stringify({
    jsonrpc: "2.0",
    id: 4,
    method: "getSlot",
    params: [],
  });

  const resp = await handleJsonRpcBody(ctx, body);

  assert.equal(resp, null, "unknown methods must return null so callers can passthrough");
});

test("handleJsonRpcBody returns null for malformed JSON", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });

  const resp = await handleJsonRpcBody(ctx, "{not json");

  assert.equal(resp, null);
});

test("createFixtureUpstream serves getAccountInfo from the accounts map", async () => {
  const upstream = createFixtureUpstream({
    accounts: {
      "SomeAddress": {
        data: new Uint8Array([1, 2, 3]),
        owner: "SomeOwner",
        lamports: 1_000_000,
      },
    },
  });

  const result = (await upstream.rpcCall("getAccountInfo", ["SomeAddress"])) as {
    value: { owner: string; lamports: number } | null;
  };

  assert.ok(result.value);
  assert.equal(result.value!.owner, "SomeOwner");
  assert.equal(result.value!.lamports, 1_000_000);
});

test("createFixtureUpstream returns null value for unknown addresses", async () => {
  const upstream = createFixtureUpstream();

  const result = (await upstream.rpcCall("getAccountInfo", ["Nope"])) as {
    value: unknown;
  };

  assert.equal(result.value, null);
});

test("createFixtureUpstream throws on un-stubbed RPC methods", async () => {
  const upstream = createFixtureUpstream();

  await assert.rejects(() => upstream.rpcCall("getBalance", ["addr"]), /no fixture/i);
});

test("createFixtureUpstream routes custom rpcResponses", async () => {
  const upstream = createFixtureUpstream({
    rpcResponses: {
      getBalance: () => ({ context: { slot: 0 }, value: 42 }),
    },
  });

  const result = (await upstream.rpcCall("getBalance", ["addr"])) as {
    value: number;
  };

  assert.equal(result.value, 42);
});
