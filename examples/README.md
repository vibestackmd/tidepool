# Tidepool examples

Runnable end-to-end demos of the four consumer patterns Tidepool supports.

## 1. MSW integration (`./msw-integration/`)

JavaScript test suite using Tidepool through MSW. Shows native-dispatch, error handling, and passthrough. The "oh my god, this just works" pattern that motivates the napi bridge.

```bash
cd examples/msw-integration && pnpm install && pnpm test
```

## 2. Rust library integration (`./rust-integration/`)

Rust consumer composing the service layer directly — seed a Bubblegum tree, mint two cNFTs, pull a DAS proof. No HTTP, no CLI, just `use tidepool_rpc::...` async fns.

```bash
cargo run -p tidepool-rpc-example-rust-integration
```

## 3. Standalone CLI proxy

Not a separate example — just the CLI itself:

```bash
# Terminal 1: start Surfpool
surfpool start

# Terminal 2: start Tidepool pointing at it
cargo run -p tidepool-rpc-cli -- start \
  --port 8897 \
  --upstream http://127.0.0.1:8899 \
  --index-tree <your-bubblegum-tree>

# Terminal 3: hit it
curl -X POST http://localhost:8897 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"surfpoolHeliusInfo","params":{}}'
```

Post-1.0, this becomes `npx tidepool-rpc start ...` via the published CLI binary.

## 4. Compose with `surfpool-sdk-node`

Pattern for in-process integration tests that embed Surfpool:

```ts
import { Surfnet } from "surfpool-sdk";
import { HeliusContext, handleJsonRpcBody } from "tidepool-rpc";

const surfnet = Surfnet.start();
const ctx = new HeliusContext({ upstreamUrl: surfnet.rpcUrl });

// In a vitest setup file — intercept requests your app makes,
// route through Tidepool, fall back to surfnet.rpcUrl for anything
// Tidepool doesn't handle natively.
```

Once the Surfpool SDK ships v1 on npm, this pattern lands as a full runnable example alongside the others. Today it's blocked on the SDK's pre-release status (they're still iterating on the napi surface).

## How these relate to the pitch

- **MSW example** — the differentiator. Nothing else in the Solana space plugs into JS test suites this cleanly.
- **Rust example** — the "I could hire this person" signal for Surfpool / Helius infra conversations.
- **CLI** — the "npm install and run" experience app devs expect.
- **Surfpool SDK** — the emerging pattern the Surfpool team is actively promoting; we meet them where they're going.
