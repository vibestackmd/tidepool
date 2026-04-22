// Nock compat. Nock intercepts Node's http(s) client, so we disable native
// fetch (which uses undici internally) for this test and drive requests
// through node's `http.request` equivalent via a shim — easier path is to
// let Nock intercept `fetch`, which it does on Node 20+ when it patches
// undici's global dispatcher.

import { test } from "node:test";
import assert from "node:assert/strict";
import nock from "nock";
import {
  createHeliusContext,
  createFixtureUpstream,
  handleJsonRpcBody,
} from "../../src/index.js";

const UPSTREAM_HOST = "http://127.0.0.1:8899";

test("Nock integration: a surfpool-helius-backed scope intercepts known methods", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });

  const scope = nock(UPSTREAM_HOST)
    .persist()
    .post("/")
    .reply(async (_uri, requestBody) => {
      const resp = await handleJsonRpcBody(ctx, requestBody as Record<string, unknown>);
      if (resp) return [200, resp];
      // Nock returns 404 by default on unmatched; we surface a JSON-RPC-ish
      // error so the test can distinguish "we returned null" from "we
      // crashed." In real code, consumers call `.allowUnmocked()` on the
      // scope and nock will let the request hit the real target.
      return [501, { jsonrpc: "2.0", id: null, error: { code: -32601, message: "passthrough" } }];
    });

  try {
    const res = await fetch(UPSTREAM_HOST, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "surfpoolHeliusInfo",
        params: [],
      }),
    });
    const json = (await res.json()) as { result: { name: string } };
    assert.equal(json.result.name, "surfpool-helius");
  } finally {
    scope.persist(false);
    nock.cleanAll();
  }
});

test("Nock integration: unknown methods yield the passthrough signal", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });

  let sawPassthrough = false;
  const scope = nock(UPSTREAM_HOST)
    .persist()
    .post("/")
    .reply(async (_uri, requestBody) => {
      const resp = await handleJsonRpcBody(ctx, requestBody as Record<string, unknown>);
      if (resp) return [200, resp];
      sawPassthrough = true;
      return [200, { ok: true }];
    });

  try {
    await fetch(UPSTREAM_HOST, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 2, method: "getSlot", params: [] }),
    });
    assert.equal(sawPassthrough, true, "expected handleJsonRpcBody to return null for unknown method");
  } finally {
    scope.persist(false);
    nock.cleanAll();
  }
});
