// Smoke test for the napi bridge. Loads the .node addon built by
// `pnpm run build`, exercises the three exported primitives, and
// asserts shape of each. Runs via `node --test __test__`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { HeliusContext, handleJsonRpcBody, version } from "../index.js";

test("version returns a non-empty string", () => {
  const v = version();
  assert.equal(typeof v, "string");
  assert.ok(v.length > 0, "version should not be empty");
});

test("HeliusContext constructs with defaults", () => {
  const ctx = new HeliusContext();
  assert.ok(ctx instanceof HeliusContext);
});

test("HeliusContext accepts options", () => {
  const ctx = new HeliusContext({
    upstreamUrl: "http://127.0.0.1:9999",
    rpcTimeoutMs: 5_000,
  });
  assert.ok(ctx instanceof HeliusContext);
});

test("handleJsonRpcBody dispatches surfpoolHeliusInfo and returns JSON", async () => {
  const ctx = new HeliusContext();
  const body = JSON.stringify({
    jsonrpc: "2.0",
    id: 1,
    method: "surfpoolHeliusInfo",
    params: {},
  });
  const response = await handleJsonRpcBody(ctx, body);
  assert.ok(response, "should return a response string");
  const parsed = JSON.parse(response);
  assert.equal(parsed.id, 1);
  assert.equal(parsed.result.name, "tidepool-rpc");
  assert.ok(Array.isArray(parsed.result.methods));
});

test("handleJsonRpcBody returns null for unknown methods (passthrough signal)", async () => {
  const ctx = new HeliusContext();
  const body = JSON.stringify({
    jsonrpc: "2.0",
    id: 2,
    method: "getSlot",
    params: [],
  });
  const response = await handleJsonRpcBody(ctx, body);
  assert.equal(response, null, "unknown method should signal passthrough");
});

test("handleJsonRpcBody returns null for malformed JSON", async () => {
  const ctx = new HeliusContext();
  const response = await handleJsonRpcBody(ctx, "{not json");
  assert.equal(response, null);
});

test("handleJsonRpcBody returns an error envelope for missing required params", async () => {
  const ctx = new HeliusContext();
  const body = JSON.stringify({
    jsonrpc: "2.0",
    id: 3,
    method: "getAssetProof",
    // No params.id → server should error.
    params: {},
  });
  const response = await handleJsonRpcBody(ctx, body);
  assert.ok(response);
  const parsed = JSON.parse(response);
  assert.ok(parsed.error, "missing param → JSON-RPC error");
  assert.equal(parsed.error.code, -32602);
});
