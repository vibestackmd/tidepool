# MSW integration — vitest + @tidepool/rpc

Full runnable demo of using Tidepool in a JavaScript test suite via MSW. This is the pattern that makes Tidepool compose cleanly with any mock-HTTP setup you already use.

## Run

```bash
cd examples/msw-integration
pnpm install           # pulls MSW + vitest, links @tidepool/rpc from ../../crates/node
pnpm test
```

You should see three passing tests covering:

1. **Native dispatch** — `surfpoolHeliusInfo` resolves in-process through `handleJsonRpcBody`, no network.
2. **Structured error** — `getAssetProof` on an unindexed id returns a proper JSON-RPC error envelope rather than crashing.
3. **Passthrough** — `getSlot` (not a Tidepool method) returns `null` from `handleJsonRpcBody`, MSW's own handlers take over.

## The shape

Three lines in your MSW setup:

```ts
import { HeliusContext, handleJsonRpcBody } from "@tidepool/rpc";
import { http, HttpResponse, passthrough } from "msw";
import { setupServer } from "msw/node";

const ctx = new HeliusContext();

const server = setupServer(
  http.post("http://127.0.0.1:8899/", async ({ request }) => {
    const body = await request.text();
    const response = await handleJsonRpcBody(ctx, body);
    return response ? HttpResponse.json(JSON.parse(response)) : passthrough();
  }),
);
```

That's the whole integration. `handleJsonRpcBody` returning `null` is your signal to defer to MSW's passthrough; any method Tidepool handles natively comes back as a serialized JSON-RPC envelope.

## Same pattern works with

- **Nock** — `.persist().post("/").reply(async (_, body) => ...)`.
- **undici MockAgent** — `.intercept({ path, method: "POST" }).reply(200, async (opts) => ...)`.
- **Any Node HTTP-mock library** — the integration contract is just "pass the body string, forward the response or fall through on null."

The only library-specific differences are how you express passthrough (MSW has `passthrough()`, Nock needs `.allowUnmocked()` on the scope, undici has `enableNetConnect`). Tidepool's contract doesn't care.

## Note on `file:` dep

The `package.json` links `@tidepool/rpc` via `file:../../crates/node` because this repo isn't published yet. In your real project after `v1.0.0` lands on npm, you'll just write `"@tidepool/rpc": "^1"`.
