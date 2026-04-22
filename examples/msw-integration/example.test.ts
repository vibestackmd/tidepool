// Example vitest tests that exercise Tidepool through MSW.
//
// What this demonstrates:
//
//   1. A JSON-RPC call to `surfpoolHeliusInfo` — a method Tidepool
//      handles natively — hits MSW, gets dispatched in-process, and
//      returns a proper JSON-RPC envelope. No network.
//
//   2. A JSON-RPC call to `getAssetProof` with a missing asset —
//      Tidepool returns a structured error (no panic, no empty
//      response). Useful for testing error paths in client code.
//
//   3. A passthrough case — `getSlot` isn't a Tidepool-native method,
//      so `handleJsonRpcBody` returns null and MSW's `passthrough()`
//      kicks in. We stub it with a second MSW handler to show the
//      integration is clean.

import { beforeAll, afterAll, afterEach, describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "./setup.js";

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

const RPC_URL = "http://127.0.0.1:8899/";

async function rpc(method: string, params: unknown = {}, id: number = 1) {
  const res = await fetch(RPC_URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
  });
  return res.json();
}

describe("Tidepool + MSW", () => {
  it("dispatches surfpoolHeliusInfo natively without hitting the network", async () => {
    const response = (await rpc("surfpoolHeliusInfo")) as {
      id: number;
      result: { name: string; methods: { method: string }[] };
    };

    expect(response.id).toBe(1);
    expect(response.result.name).toBe("tidepool-rpc");
    expect(response.result.methods.length).toBeGreaterThan(5);
    expect(response.result.methods.some((m) => m.method === "getAssetProof")).toBe(true);
  });

  it("returns a structured error for getAssetProof on an unindexed tree", async () => {
    const response = (await rpc("getAssetProof", {
      id: "11111111111111111111111111111111",
    })) as { id: number; error?: { code: number; message: string } };

    expect(response.id).toBe(1);
    expect(response.error).toBeDefined();
    expect(response.error!.code).toBe(-32000);
  });

  it("falls through to MSW for unknown methods (getSlot)", async () => {
    // Register a second handler that intercepts after Tidepool passes.
    server.use(
      http.post(RPC_URL, async ({ request }) => {
        const body = (await request.json()) as { method: string; id: number };
        if (body.method === "getSlot") {
          return HttpResponse.json({
            jsonrpc: "2.0",
            id: body.id,
            result: 424242,
          });
        }
        // Everything else goes to the Tidepool-backed handler in setup.ts.
        return undefined;
      }),
    );

    const response = (await rpc("getSlot", [])) as { result: number };
    expect(response.result).toBe(424242);
  });
});
