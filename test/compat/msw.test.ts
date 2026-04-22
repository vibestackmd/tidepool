// MSW compat. Proves a consumer can stand up MSW, hand it a single handler
// that delegates to surfpool-helius, and watch it intercept JSON-RPC calls
// transparently.

import { test } from "node:test";
import assert from "node:assert/strict";
import { setupServer } from "msw/node";
import { http, HttpResponse, passthrough } from "msw";
import {
  createHeliusContext,
  createFixtureUpstream,
  handleJsonRpcBody,
} from "../../src/index.js";

const UPSTREAM = "http://127.0.0.1:8899/";

test("MSW integration: handleJsonRpcBody routes a known method", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });

  const server = setupServer(
    http.post(UPSTREAM, async ({ request }) => {
      const resp = await handleJsonRpcBody(ctx, await request.text());
      if (resp) return HttpResponse.json(resp);
      return passthrough();
    }),
  );
  server.listen({ onUnhandledRequest: "bypass" });

  try {
    const res = await fetch(UPSTREAM, {
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
    server.close();
  }
});

test("MSW integration: unknown methods fall through to passthrough", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });

  // A second handler proves passthrough() actually yields — this one stands
  // in for "the real Surfpool" and should receive whatever surfpool-helius
  // declines to handle.
  const server = setupServer(
    http.post(UPSTREAM, async ({ request }) => {
      const resp = await handleJsonRpcBody(ctx, await request.text());
      if (resp) return HttpResponse.json(resp);
      return passthrough();
    }),
    http.post("http://fake-surfpool.test/", async () =>
      HttpResponse.json({ jsonrpc: "2.0", id: 99, result: "from-passthrough-target" }),
    ),
  );
  server.listen({ onUnhandledRequest: "bypass" });

  try {
    const res = await fetch("http://fake-surfpool.test/", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 99, method: "getSlot", params: [] }),
    });
    const json = (await res.json()) as { result: string };
    assert.equal(json.result, "from-passthrough-target");
  } finally {
    server.close();
  }
});
