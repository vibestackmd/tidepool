// undici MockAgent compat. This is the "no third-party test library" path
// — anyone using Node's native fetch can use undici's own MockAgent
// without adding MSW or Nock. Node's built-in fetch uses the internal
// undici, so the test imports `fetch` from the installed undici package
// to exercise the MockAgent we set up.

import { test } from "node:test";
import assert from "node:assert/strict";
import { MockAgent, fetch as undiciFetch } from "undici";
import {
  createHeliusContext,
  createFixtureUpstream,
  handleJsonRpcBody,
} from "../../src/index.js";

const ORIGIN = "http://127.0.0.1:8899";

test("undici MockAgent integration: intercepts known methods", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });

  const agent = new MockAgent();
  agent.disableNetConnect();
  const pool = agent.get(ORIGIN);

  pool
    .intercept({ path: "/", method: "POST" })
    .reply(200, async (opts) => {
      // `opts.body` is a string when the client sent a string body, which
      // is what `fetch` does for a JSON-serialized body.
      const raw = typeof opts.body === "string" ? opts.body : "";
      const resp = await handleJsonRpcBody(ctx, raw);
      return resp ?? { jsonrpc: "2.0", id: null, error: { code: -32601, message: "passthrough" } };
    }, { headers: { "content-type": "application/json" } })
    .persist();

  try {
    const res = await undiciFetch(ORIGIN, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "surfpoolHeliusInfo",
        params: [],
      }),
      dispatcher: agent,
    });
    const json = (await res.json()) as { result: { name: string } };
    assert.equal(json.result.name, "surfpool-helius");
  } finally {
    await agent.close();
  }
});
