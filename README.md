# surfpool-helius

**Helius, running on your laptop.** Drop-in Helius-compatible local dev environment on top of [Surfpool](https://github.com/solana-foundation/surfpool). Point your app at it, develop against mainnet-shaped data for free, ship to production Helius without changing a line of client code.

## What you get

Three things that are obvious the moment you try them:

### 1. Local DAS, including compressed NFTs

`getAsset`, `getAssetBatch`, `getAssetProof`, `getAssetsByOwner`, `getAssetsByGroup`, `getAssetsByAuthority`, `getAssetsByCreator`, `searchAssets`, `getNftEditions`. MplCore, Token Metadata (Token / Token-2022), and Bubblegum cNFTs — all resolved locally from real on-chain bytes. No Helius key, no rate limits, no cost, no flaky internet.

Compressed NFTs go through a full local Bubblegum indexer: mint, transfer, burn, delegate, verifyCreator / unverifyCreator, verifyCollection / unverifyCollection / setAndVerifyCollection, updateMetadata. Authoritative state comes from the noop-CPI LeafSchemaEvent, so proofs match on-chain even after a `setAndVerifyCollection` — the realistic case that breaks simpler cNFT tools.

### 2. `confirmTransaction()` actually works on Surfpool

Surfpool's native WebSocket doesn't implement `signatureSubscribe`, which means `@solana/web3.js`'s `confirmTransaction()` and `sendAndConfirm()` hang forever. We polyfill it via HTTP polling. Every `helius-sdk` method that composes "send, wait, assert" — `sendSmartTransaction`, `broadcastTransaction`, `pollTransactionConfirmation` — just works. This alone is worth running the proxy even if you don't touch DAS.

### 3. Plugs into MSW, Nock, or undici for tests — zero new deps

Transport-agnostic core. `import { createHeliusContext, handleJsonRpcBody } from "surfpool-helius"`, plug the result into any mock-HTTP library you already use, and your test suite gets deterministic Helius-compatible responses without a validator, without the network. Works with MSW, Nock, undici's MockAgent, or anything else that intercepts HTTP.

## Quickstart

You need [Surfpool](https://github.com/solana-foundation/surfpool) and Node 20+.

```bash
# 1. Start Surfpool
surfpool start                                            # native install
docker run --rm -p 8899:8899 -p 8900:8900 \
  surfpool/surfpool:latest start --no-tui --host=0.0.0.0  # or Docker

# 2. Run the proxy
npx surfpool-helius
```

Point your app at the proxy:

```ts
import { Connection } from "@solana/web3.js";
const connection = new Connection("http://localhost:8897", "confirmed");
```

```bash
curl -X POST http://localhost:8897 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getAsset","params":{"id":"<asset-address>"}}'
```

For cNFTs, register the tree you want to index:

```bash
npx surfpool-helius --index-tree <merkle-tree-pubkey>
```

Or programmatically at runtime:

```bash
curl -X POST http://localhost:8897 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"surfpoolHeliusIndexTree","params":{"tree":"<merkle-tree>"}}'
```

That's it.

## Architecture

```
  ┌─────────────┐        ┌──────────────────┐        ┌───────────┐
  │   Your app  │───────▶│  surfpool-helius │───────▶│  Surfpool │
  │  RPC_URL =  │        │  HTTP :8897      │        │   :8899   │
  │   :8897     │        │    WS :8898      │        │   WS :8900│
  └─────────────┘        │                  │        └───────────┘
                         │  DAS (MplCore,   │
                         │  Token Metadata, │
                         │  cNFTs):         │              ▲
                         │  • getAsset      │              │
                         │  • getAssetProof │              │
                         │  • searchAssets  │              │
                         │                  │              │
                         │  WS polyfill:    │              │
                         │  • sigSubscribe  │──────────────┘
                         │                  │   (HTTP polling)
                         │  Everything else │
                         │  → passthrough   │
                         └──────────────────┘
```

`getAsset` on an uncompressed asset fetches the raw account from the upstream via `getAccountInfo`, runs it through a pluggable decoder, and returns a DAS-shaped response. Every asset that passes through is indexed by owner, authority, creator, and grouping, so `searchAssets`, `getAssetsByOwner`, `getAssetsByGroup`, `getAssetsByAuthority`, and `getAssetsByCreator` all serve from that index. Assets the proxy has never fetched are invisible to the index — the documented local-dev tradeoff.

`getAsset` on a cNFT resolves from the local Bubblegum indexer: when a tree is registered, we walk `getSignaturesForAddress` for that tree, parse every Bubblegum ix (outer + inner, including CPI wrappers like Candy Guard), track authoritative state via the noop LeafSchemaEvent, and serve `getAsset` / `getAssetProof` straight from the replayed state.

## Supported methods

Full live truth: `POST {"method":"surfpoolHeliusInfo"}` against a running proxy. The tables below are regenerated from it.

### DAS

| Method                  | Status        | Notes                                                                 |
|-------------------------|---------------|-----------------------------------------------------------------------|
| `getAsset`              | ✅ v0.1       | MplCore, Token Metadata (incl. Token-2022), cNFTs (since v0.6)        |
| `getAssetBatch`         | ✅ v0.2       | Up to 1000 ids; parallel upstream reads                               |
| `getAssetProof`         | ✅ v0.6       | cNFTs — requires tree to be registered via `indexTrees` or runtime    |
| `getAssetProofBatch`    | ✅ v0.6       | Parallel over `getAssetProof`; unknown ids → null                     |
| `getAssetsByOwner`      | ✅ v0.2       | Serves from the local index                                           |
| `getAssetsByGroup`      | ✅ v0.2       | MplCore: `groupKey: "collection"`                                     |
| `getAssetsByAuthority`  | ✅ v0.2       | MplCore update authority                                              |
| `getAssetsByCreator`    | ✅ v0.3       | Derived from Royalties + VerifiedCreators plugins                     |
| `searchAssets`          | ✅ v0.3       | Full filter surface including `creatorAddress`                        |
| `getNftEditions`        | ✅ v0.5       | Master supply is exact; editions[] is local-indexed                   |

### WebSocket + tx ergonomics

| Method                       | Status  | Notes                                                             |
|------------------------------|---------|-------------------------------------------------------------------|
| `signatureSubscribe`         | ✅ v0.1 | Polyfilled via HTTP polling — `confirmTransaction()` actually works |
| `signatureUnsubscribe`       | ✅ v0.1 | Cancels the polling timer                                         |
| `getPriorityFeeEstimate`     | ✅ v0.4 | Local percentiles over `getRecentPrioritizationFees`              |
| `getProgramAccountsV2`       | ✅ v0.4 | Cursor-paginated passthrough                                      |
| `getTokenAccountsByOwnerV2`  | ✅ v0.4 | Cursor-paginated passthrough                                      |

### Also works

- **`helius-sdk` transparent composition** — `helius.tx.sendSmartTransaction`, `helius.staking.*`, `helius.wallet.*`, and every other SDK helper that's built from standard RPC calls. Point `helius-sdk` at the proxy and it just works. See [Using `helius-sdk`](#using-helius-sdk).
- **Everything else** — any method the proxy doesn't handle is forwarded to the upstream unchanged.

### Roadmap

Enhanced Transactions parsing (`getTransactions`, `getTransactionsByAddress`), Webhooks simulator, Wallet API, additional WS subscriptions (account/logs/program/slot/root). V2 Bubblegum ix family (`mintV2`, `transferV2`, etc.). See `surfpoolHeliusInfo` → `methods[]` filtered on `compat: "PLANNED"` for the full list.

## Plugging into MSW, Nock, or undici MockAgent

If your team already has a mock-HTTP layer — MSW in component tests, Nock in backend tests, undici's MockAgent anywhere native `fetch` runs — surfpool-helius plugs in directly. The library ships a transport-agnostic core (`createHeliusContext` + `handleJsonRpcBody`) that returns a JSON-RPC response for any method it implements, or `null` when it doesn't. You wire that into whichever library's handler shape you already use.

**Zero runtime dependency on MSW, Nock, or undici** — you install what you already use; this library stays neutral.

**One-time setup**, shared across every example below:

```ts
import { createHeliusContext, createFixtureUpstream } from "surfpool-helius";

const ctx = createHeliusContext({
  upstream: createFixtureUpstream({
    // accounts: { "<pubkey>": { data, owner, lamports } }
  }),
});
```

**MSW:**

```ts
import { setupServer } from "msw/node";
import { http, HttpResponse, passthrough } from "msw";
import { handleJsonRpcBody } from "surfpool-helius";

const server = setupServer(
  http.post("http://127.0.0.1:8899/", async ({ request }) => {
    const resp = await handleJsonRpcBody(ctx, await request.text());
    return resp ? HttpResponse.json(resp) : passthrough();
  }),
);
server.listen();
```

**Nock:**

```ts
import nock from "nock";
import { handleJsonRpcBody } from "surfpool-helius";

nock("http://127.0.0.1:8899")
  .persist()
  .post("/")
  .reply(async (_uri, body) => {
    const resp = await handleJsonRpcBody(ctx, body as Record<string, unknown>);
    return resp ? [200, resp] : [501, { error: "method not handled" }];
  });
// Chain `.allowUnmocked()` on the scope to let unknowns hit the real upstream.
```

**undici MockAgent** (native `fetch`, no extra library):

```ts
import { MockAgent, setGlobalDispatcher } from "undici";
import { handleJsonRpcBody } from "surfpool-helius";

const agent = new MockAgent();
setGlobalDispatcher(agent);
agent
  .get("http://127.0.0.1:8899")
  .intercept({ path: "/", method: "POST" })
  .reply(200, async (opts) => {
    const resp = await handleJsonRpcBody(ctx, String(opts.body));
    return resp ?? { error: "passthrough" };
  })
  .persist();
```

**Caveats:**

- Passthrough semantics differ per library. `handleJsonRpcBody` returning `null` is your signal to defer.
- HTTP only. `signatureSubscribe` is available only through `createProxy` (the built-in server).
- One context per mock instance — cache is stateful; cross-test pollution will surprise you.

## Using `helius-sdk`

Most of `helius-sdk`'s surface is client-side composition over standard wire methods. These work transparently against this proxy because the proxy handles the underlying wire RPC calls:

```ts
import { createHelius } from "helius-sdk";

const helius = createHelius("any-key", { baseUrl: "http://localhost:8897" });

// All of this works locally:
await helius.tx.sendSmartTransaction(instructions, signers);
await helius.das.getAsset({ id });
await helius.das.getAssetProof({ id });
await helius.staking.createStakeTransaction({ amount });
```

The compatibility manifest marks these methods as `SDK_WRAPPER`. `POST {"method":"surfpoolHeliusInfo"}` returns every method grouped by compat level: `EXACT`, `LOCAL_INDEX`, `BEST_EFFORT`, `SHIM`, `SDK_WRAPPER`, `PLANNED`, `SKIPPED`.

## Configuration

All options are optional; defaults target a standard local Surfpool install.

**CLI:**

```bash
npx surfpool-helius \
  --port 8897 \
  --upstream http://127.0.0.1:8899 \
  --upstream-ws ws://127.0.0.1:8900 \
  --index-tree <merkle-tree-pubkey>      # repeatable
```

**Environment variables:**
`SURFPOOL_HELIUS_PORT`, `SURFPOOL_HELIUS_UPSTREAM_URL`, `SURFPOOL_HELIUS_UPSTREAM_WS_URL`, `SURFPOOL_HELIUS_INDEX_TREES` (comma-separated).

**Programmatic:**

```ts
import { createProxy, mplCoreDecoder, tokenMetadataDecoder } from "surfpool-helius";

await createProxy({
  port: 8897,
  upstreamUrl: "http://127.0.0.1:8899",
  upstreamWsUrl: "ws://127.0.0.1:8900",
  rpcTimeoutMs: 10_000,
  decoders: [mplCoreDecoder, tokenMetadataDecoder],
  indexTrees: ["<merkle-tree-pubkey>"],
});
```

## Pluggable decoders

The built-in MplCore and Token Metadata decoders cover almost every real NFT in the wild. To add another program, implement `AccountDecoder`:

```ts
import { createProxy, mplCoreDecoder } from "surfpool-helius";
import type { AccountDecoder } from "surfpool-helius";

const myDecoder: AccountDecoder = {
  programId: "YourProgram111111111111111111111111111111111",
  name: "my-program",
  async decode(pubkey, data) {
    // Parse `data`, return a DAS-shaped asset or null.
    return null;
  },
};

await createProxy({ decoders: [mplCoreDecoder, myDecoder] });
```

The proxy picks a decoder by matching the account's owner program ID — first match wins. Pass `decoders: []` to disable account-based DAS entirely (cNFTs still work — they don't go through decoders).

## Compressed NFTs (cNFTs)

cNFTs don't live on-chain as accounts — they're leaves in a Bubblegum merkle tree. There's no account to fetch, no decoder to run. surfpool-helius ships a local Bubblegum indexer that replays every tree-mutating transaction into an in-memory store, from which `getAsset` / `getAssetProof` serve directly.

**Registering a tree:**

```bash
# At startup:
npx surfpool-helius --index-tree <merkle-tree-pubkey>

# At runtime (e.g. in test setup):
curl -X POST http://localhost:8897 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"surfpoolHeliusIndexTree","params":{"tree":"<merkle-tree>"}}'
```

**Tracked instructions:** `createTree`, `mintV1`, `mintToCollectionV1`, `transfer`, `burn`, `delegate`, `verifyCreator`, `unverifyCreator`, `verifyCollection`, `unverifyCollection`, `setAndVerifyCollection`, `updateMetadata`. For hash-dependent ixs, authoritative new state comes from the noop LeafSchemaEvent CPI, so proofs match on-chain even after multi-step flows like mint → setAndVerifyCollection.

**Not yet tracked:** V2 ix family (`mintV2`, `transferV2`, `burnV2`, …). cNFTs minted via the V2 path require the V2 extension — roadmap.

## FAQ

**Is this production-ready?** No. It's a local development tool. Ship to real Helius in production.

**Does this replace Helius?** No. It lets you develop against Helius locally so your production integration has a tight dev loop.

**Is this endorsed by Helius?** No — it's a community tool. Helius is great and you should use them.

**Why not depend on `@metaplex-foundation/mpl-core` / `mpl-bubblegum`?** Those packages are UMI-flavored and pull in a large tree. For a lean proxy we hand-roll Borsh decoding via Codama-generated Kit-native clients from pinned IDLs. The `AccountDecoder` interface is pluggable — drop in any decoder you prefer.

**Can I use this with `litesvm` or `solana-test-validator` instead of Surfpool?** Yes. The proxy just needs something that speaks standard Solana RPC. Point `--upstream` at any RPC endpoint. Surfpool is the default because it forks mainnet on demand — any mainnet account "just works" without pre-cloning. Other validators work, but require you to load accounts explicitly.

**Does this work with Solana RPC 2.0 / Triton's new read layer?** Yes. surfpool-helius proxies standard JSON-RPC, which [RPC 2.0](https://blog.triton.one/announcing-rpc-2-0-with-solana-foundation-rethinking-solanas-read-layer-from-the-ground-up/) preserves for backward compatibility. DAS isn't part of RPC 2.0, so the local dev gap this tool fills stays relevant.

## Example

[`examples/mint-and-query`](examples/mint-and-query) — full end-to-end: point UMI at the proxy, mint an MplCore asset, wait for confirmation (exercising the `signatureSubscribe` polyfill), fetch it back via `getAsset`.

## Roadmap

- **v0.1** — MplCore decoder, DAS core, `signatureSubscribe` polyfill, pass-through proxy, compatibility manifest
- **v0.2** — Batching, by-owner / by-group / by-authority queries, full `searchAssets` filtering
- **v0.3** — MplCore plugin walker, creators field, `getAssetsByCreator`
- **v0.4** — `getPriorityFeeEstimate`, V2 RPC (`getProgramAccountsV2`, `getTokenAccountsByOwnerV2`), `SDK_WRAPPER` compat level
- **v0.5** — Token Metadata decoder (legacy NFTs + Token-2022), `getNftEditions`
- **v0.6** — **Compressed NFTs** via local Bubblegum indexer (`getAsset` / `getAssetProof` / `getAssetProofBatch` for cNFTs, including noop-CPI parsing for `setAndVerifyCollection` / `verifyCreator` / `updateMetadata`)
- **v0.7** — Enhanced Transactions parser, `getTransactionsForAddress`
- **v0.8** — Webhooks simulator, additional WS subscriptions, Wallet API
- **Maybe** — Rust port, standalone binary, Bubblegum V2 ix family, local [Dragon's Mouth](https://docs.triton.one/project-yellowstone/dragons-mouth-grpc-subscriptions) polyfill

## Related

- [Surfpool](https://github.com/solana-foundation/surfpool) — the local Solana validator this runs on top of
- [Helius DAS API](https://www.helius.dev/docs/api-reference/das) — the production API this mimics
- [Metaplex MplCore](https://developers.metaplex.com/core), [Bubblegum](https://developers.metaplex.com/bubblegum) — the asset standards the built-in decoders handle
- [Triton RPC 2.0](https://blog.triton.one/announcing-rpc-2-0-with-solana-foundation-rethinking-solanas-read-layer-from-the-ground-up/)

## Built by

[Tyler Buchea](https://github.com/TylerTheBuildor) — building on Solana since the Candy Machine era. I built this because I needed it, and every team on Helius has felt the same pain. [Twitter](https://twitter.com/TylerTheBuildor).

## License

MIT
