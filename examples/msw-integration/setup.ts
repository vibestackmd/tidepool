// MSW server setup. Boots once per test run.
//
// The pattern: one MSW handler forwards every POST to the canonical
// Helius/Solana RPC URL pattern through `handleJsonRpcBody`. If
// Tidepool handles the method natively, we return its JSON envelope.
// Otherwise MSW's `passthrough()` sends the request along to the real
// upstream (or, in fully-offline tests, another handler you register
// that short-circuits everything).

import { http, HttpResponse, passthrough } from "msw";
import { setupServer } from "msw/node";
import { HeliusContext, handleJsonRpcBody } from "@tidepool-rpc/node";

// One context for the whole test run. In parallel test setups you'd
// spin up one per worker; we're single-threaded here so one is enough.
export const ctx = new HeliusContext({
  // Point at whatever upstream you'd use in CI — could be a real
  // Surfpool, a test-validator, or a URL you never intend to hit
  // (which works because Tidepool's native methods don't touch the
  // upstream; only passthrough does).
  upstreamUrl: "http://127.0.0.1:8899",
});

export const server = setupServer(
  http.post("http://127.0.0.1:8899/", async ({ request }) => {
    const body = await request.text();
    const response = await handleJsonRpcBody(ctx, body);
    if (response !== null) {
      return HttpResponse.json(JSON.parse(response));
    }
    // Method we don't handle natively — let MSW forward (or the test
    // author's own handler takes over).
    return passthrough();
  }),
);
