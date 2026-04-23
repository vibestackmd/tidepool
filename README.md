# Tidepool

**Helius DAS, locally. Built on Surfpool.**

A local, Helius-compatible development environment for Solana. Serves the DAS API, the Helius SDK's wire methods, WebSocket subscriptions, and compressed-NFT merkle proofs — all from a Rust binary you run next to your Surfpool validator. Your app's production Helius integration works locally without a key, without the internet, and without cost.

Three things you'll notice in the first five minutes:

### 1. Local DAS, including compressed NFTs

`getAsset`, `getAssetBatch`, `getAssetProof`, `getAssetsByOwner`, `getAssetsByGroup`, `getAssetsByAuthority`, `getAssetsByCreator`, `searchAssets`. MplCore, Token Metadata (both Token and Token-2022), and Bubblegum cNFTs — all resolved locally from real on-chain bytes.

Compressed NFTs go through a full local Bubblegum indexer: every mint, transfer, burn, delegate, verify / unverify creator or collection, set-and-verify collection, and update-metadata instruction is replayed. Authoritative state comes from the noop-CPI `LeafSchemaEvent`, so proofs match on-chain even after a `setAndVerifyCollection` — the realistic case that breaks thinner cNFT tooling.

### 2. `confirmTransaction()` actually works on Surfpool

Surfpool's native WebSocket doesn't implement `signatureSubscribe`, which means `@solana/web3.js`'s `confirmTransaction()` and `sendAndConfirm()` hang against raw Surfpool. Tidepool polyfills it via HTTP polling. Every `helius-sdk` method that composes "send, wait, assert" — `sendSmartTransaction`, `broadcastTransaction`, `pollTransactionConfirmation` — just works. This alone is worth running even if you don't touch DAS.

### 3. Plugs into MSW / Nock / undici for tests — zero extra infrastructure

Import `tidepool-rpc` from npm, plug `handleJsonRpcBody` into whichever mock-HTTP layer your team already uses, and your test suite gets deterministic Helius responses without standing up a validator. Nothing else in this space delivers test-integration ergonomics this directly.

## Quickstart

Three ways to consume it, pick one.

### As a binary (most common)

```bash
# In one terminal:
surfpool start

# In another:
cargo install tidepool-rpc-cli          # or `npx tidepool-rpc start` post-1.0
tidepool-rpc start \
  --port 8897 \
  --upstream http://127.0.0.1:8899 \
  --index-tree <your-cNFT-merkle-tree>

# Your app:
import { Connection } from "@solana/web3.js";
const connection = new Connection("http://localhost:8897", "confirmed");
```

### As a Rust library

```toml
[dependencies]
tidepool-rpc = "1"
```

```rust
use tidepool_rpc::cnft::{apply_event, CnftEvent, MemoryCnftStore};
use tidepool_rpc::das::{get_asset, get_asset_proof};
```

See [`examples/rust-integration/`](examples/rust-integration/) for a runnable walkthrough.

### As a Node / JS test integration

```bash
npm install tidepool-rpc msw vitest
```

```ts
import { HeliusContext, handleJsonRpcBody } from "tidepool-rpc";
import { http, HttpResponse, passthrough } from "msw";
import { setupServer } from "msw/node";

const ctx = new HeliusContext();

setupServer(
  http.post("http://127.0.0.1:8899/", async ({ request }) => {
    const response = await handleJsonRpcBody(ctx, await request.text());
    return response ? HttpResponse.json(JSON.parse(response)) : passthrough();
  }),
).listen();
```

Full runnable vitest setup in [`examples/msw-integration/`](examples/msw-integration/).

### As a drop-in for `helius-sdk`

If you're already using `helius-sdk` in production, point the same client at Tidepool for local development — one URL swap, everything works. The SDK's JSON-RPC, REST (`/v0/…`), and WebSocket transports all resolve against Tidepool because we mirror Helius's transport split exactly.

```ts
import { Helius } from "helius-sdk";

const helius = new Helius(
  process.env.HELIUS_API_KEY ?? "local-dev",
  // Dev: Tidepool. Prod: omit and let the SDK default to
  // mainnet.helius-rpc.com + api.helius.xyz.
  process.env.NODE_ENV === "development"
    ? { url: "http://localhost:8897", restUrl: "http://localhost:8897" }
    : undefined,
);

// Uses JSON-RPC — hits Tidepool's local DAS index.
await helius.rpc.getAsset({ id: mintPubkey });

// Uses REST — hits Tidepool's /v0 layer.
await helius.enhanced.getTransactions([signature]);

// Uses WS — hits Tidepool's polling polyfill on :8898.
await helius.ws.signatureNotifications(signature);
```

The `restUrl` + `url` split above assumes a small PR landing in [`helius-labs/helius-sdk`](https://github.com/helius-labs/helius-sdk) (WIP) to make the REST base URL configurable. Until it merges, the JSON-RPC + WS paths work today; REST needs the SDK's internal base URL overridden via whatever escape hatch your SDK version provides (older versions allow a patched `fetch` wrapper).

## Architecture

```
  ┌─────────────┐        ┌──────────────────┐        ┌───────────┐
  │   Your app  │───────▶│     Tidepool     │───────▶│  Surfpool │
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

Uncompressed `getAsset` fetches the account from the upstream, runs it through a pluggable decoder (`mpl-core` or `mpl-token-metadata`), and returns a DAS-shaped response. The cache populates as a side effect so `searchAssets`, `getAssetsByOwner`, and the other secondary-index queries work.

Compressed `getAsset` / `getAssetProof` resolve from a local Bubblegum indexer: `getSignaturesForAddress` walks the tree, `getTransaction` pulls each candidate tx, inner Bubblegum + noop CPIs are parsed for authoritative leaf state. Trees are registered via `--index-tree` at startup or `tidepool_indexTree` at runtime.

## Why Surfpool?

Tidepool works with any Solana RPC that speaks standard wire methods — `solana-test-validator` with `--clone`, real devnet, or anything else. Surfpool is the recommended upstream for local dev because its mainnet-forking means any real account you ask about "just works" without you pre-declaring it. That makes the dev-loop feel magic instead of tedious.

The `signatureSubscribe` polyfill specifically exists because Surfpool doesn't implement it — so Tidepool delivers strictly more value when Surfpool is the upstream than when anything else is.

## Supported methods

Full live truth: `POST {"method":"tidepool_info"}` returns the complete manifest + summary. Every entry is classified `EXACT`, `LOCAL_INDEX`, `BEST_EFFORT`, `SHIM`, `SDK_WRAPPER`, `PLANNED`, or `SKIPPED`. The table below is a snapshot.

| Method | Status | Notes |
|---|---|---|
| `getAsset` / `getAssetBatch` | ✅ LOCAL_INDEX | MplCore, Token Metadata (incl. Token-2022), cNFTs |
| `getAssetProof` / `getAssetProofBatch` | ✅ LOCAL_INDEX | Requires tree registered via `--index-tree` or runtime method |
| `getAssetsByOwner` / `Authority` / `Creator` / `Group` | ✅ LOCAL_INDEX | Cache-backed secondary indexes |
| `searchAssets` | ✅ LOCAL_INDEX | Multi-filter AND, smallest-index-first narrowing |
| `getNftEditions` | ✅ LOCAL_INDEX | Lazy edition-PDA indexing; master + print editions |
| `signatureSubscribe` / `accountSubscribe` / `logsSubscribe` (+ `Unsubscribe`) | ✅ SHIM | HTTP polling polyfills on the WS port. `logsSubscribe` supports `{ mentions: [pubkey] }`; `all` / `allWithVotes` intentionally unsupported. |
| `getPriorityFeeEstimate` | ✅ BEST_EFFORT | Local percentile ladder over `getRecentPrioritizationFees` |
| `helius-sdk` composed methods | ✅ SDK_WRAPPER | Send / broadcast / confirm / staking — all work transparently |
| `getBalances` (REST) | ✅ SHIM | `GET /v0/addresses/{addr}/balances` — matches Helius REST path |
| `getTransactions` / `getTransactionsByAddress` (REST) | ✅ SHIM | Enhanced Transactions parsers on `/v0/transactions` and `/v0/addresses/{addr}/transactions` |
| `createWebhook` family (REST) | ✅ SHIM | Polling-simulator on `/v0/webhooks` + `/v0/webhooks/{id}` — full CRUD |
| Everything else | ✅ Passthrough | Forwarded to the upstream unchanged |

### Transports

Tidepool matches Helius's transport split exactly — a method lives where Helius puts it, and nowhere else. That way you can't write local-dev code that'd break against production.

- **JSON-RPC** (`POST /`): DAS (`getAsset*`), Bubblegum tree control (`tidepool_*`), standard RPC passthrough.
- **REST** (`/v0/…`): Wallet (`getBalances`), Enhanced Transactions, Webhooks CRUD. Same paths as `api.helius.xyz/v0/...`.
- **WebSocket** (`ws://host:port+1`): `signatureSubscribe`, `accountSubscribe`, `signatureUnsubscribe`.
- **SDK Wrapper**: `sendSmartTransaction`, `broadcastTransaction`, `pollTransactionConfirmation`, stake/unstake — composed methods from `helius-sdk`, no wire method of their own.

`tidepool_info` returns a `transport` field per method so tooling can introspect without guessing.

## Compressed NFTs (cNFTs)

cNFTs live as leaves in a Bubblegum merkle tree, not as standalone accounts. Tidepool ships a local indexer that replays every tree-mutating instruction into an in-memory store, from which `getAsset` / `getAssetProof` serve directly.

Register a tree:

```bash
# At startup:
tidepool-rpc start --index-tree <merkle-tree>

# At runtime (in a vitest setup file, CI script, etc.):
curl -X POST http://localhost:8897 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tidepool_indexTree","params":{"tree":"<merkle-tree>"}}'
```

**Tracked:** `createTree`, `mintV1` / `mintV2`, `mintToCollectionV1`, `transfer` / `transferV2`, `burn` / `burnV2`, `delegate` / `delegateV2`, `verifyCreator` / `verifyCreatorV2`, `unverifyCreator`, `verifyCollection`, `unverifyCollection`, `setAndVerifyCollection` / `setCollectionV2`, `updateMetadata` / `updateMetadataV2`. For hash-dependent ixs, authoritative state comes from the noop `LeafSchemaEvent` CPI — proofs stay correct through multi-step flows. Covers both SPL-NOOP (V1) and MPL-NOOP (V2) noop programs.

## Persistence

Default behavior is **in-memory** — state is lost on restart. Two flags turn that off, shaped after [Surfpool's own persistence UX](https://github.com/txtx/surfpool) so the two tools feel familiar:

```bash
# Single SQLite file holds cNFT index + DAS cache + webhook registry.
tidepool-rpc start --db ./tidepool.sqlite

# Explicit ephemeral SQLite (rarely useful — same as omitting the flag).
tidepool-rpc start --db :memory:

# Preload snapshot(s) at boot. Repeatable. File format is the envelope
# returned by tidepool_exportTreeSnapshot.
tidepool-rpc start --snapshot ./trees/foo.json --snapshot ./trees/bar.json
```

`tidepool_exportTreeSnapshot` dumps a tree's indexed state at runtime; write it to a file, commit to your repo, and every fresh boot can `--snapshot` that file to skip re-paging tx history. Shape parallels Surfpool's `surfnet_exportSnapshot` but scoped to cNFT trees (different data model).

## Examples

- [`examples/msw-integration/`](examples/msw-integration/) — vitest + MSW + Tidepool, three runnable tests
- [`examples/rust-integration/`](examples/rust-integration/) — cargo example composing the service layer directly
- [`examples/README.md`](examples/) — all four consumer patterns indexed

## Workspace layout

| Crate | Purpose |
|---|---|
| `tidepool-rpc-core` | Pure primitives: keccak, merkle math, LeafSchemaV1 hashing, proof compute/verify. Zero Solana deps — WASM-ready. |
| `tidepool-rpc` | Service layer: cNFT state machine, DAS handlers, cache, decoders, upstream trait. Depends on `mpl-core` / `mpl-token-metadata` / `mpl-bubblegum`. |
| `tidepool-rpc-server` | axum HTTP + WS front-end. Method-enum dispatch. `HttpUpstream` via reqwest. |
| `tidepool-rpc-cli` | `tidepool-rpc` binary. clap-derive args + env-var overlay. |
| `tidepool-rpc-node` | napi-rs bridge → npm package `tidepool-rpc`. |

Library consumers pull `tidepool-rpc`. Binary users `cargo install tidepool-rpc-cli`. JS users `npm install tidepool-rpc`. Server builders compose `tidepool-rpc` + `tidepool-rpc-server::HttpUpstream` themselves.

## FAQ

**Is this production-ready?** No. It's a local development tool. Ship to real Helius in production.

**Does this replace Helius?** No. It lets you develop against Helius's API locally so your production integration has a tight dev loop.

**Is this endorsed by Helius or Surfpool?** Community tool, no official endorsement. Both are great companies and you should use them.

**Why not just hit real Helius in dev?** You'd burn rate limits, pollute prod monitoring, require internet on CI, and can't test without an API key. Tidepool is the answer to "I want the dev loop to be instant + offline."

**Can I use this with `solana-test-validator` or `litesvm`?** `solana-test-validator` works — point `--upstream` at it, clone mainnet accounts you care about via `--clone`. `litesvm` is in-process-only, so there's no RPC endpoint for Tidepool to proxy. Use Surfpool for the magic, test-validator for the boring-but-predictable case.

**Why Rust?** The previous version was TypeScript (v0.6, preserved at that tag). The Rust rewrite earned: drop-in `mpl-core` / `mpl-token-metadata` / `mpl-bubblegum` official crates (no Codama pipeline), exhaustive-match method dispatch (compile-time safety for adding new handlers), type-level noop-required-vs-optional enforcement on cNFT events, binary distribution via `cargo install`. And the napi-rs bridge means JS consumers still get the test-integration story via `npm install tidepool-rpc` — one Rust core, two consumption ecosystems.

**Does the WS polyfill work over compressed transactions?** The polyfill polls `getSignatureStatuses` which resolves any signature the validator has seen. Works for compressed + uncompressed transactions identically.

## Roadmap

- **v1.0** — Rust rewrite with MplCore / Token Metadata / cNFT decoders, full DAS surface, WS `signatureSubscribe` polyfill, axum server, CLI binary, napi bridge
- **v1.1** — `getNftEditions` (edition-PDA indexing), V2 Bubblegum ixs, Token Metadata owner resolution
- **v1.2** — Enhanced Transactions parsers, `getTransactionsForAddress`
- **v1.3** — Webhooks simulator, additional WS subscriptions (account / logs / program), Wallet API
- **Maybe** — Dragon's Mouth (Yellowstone gRPC) polyfill, persistent SQLite-backed stores

## Versions

- **v0.1–v0.6** — TypeScript implementation. Preserved at tags `v0.1.0` through `v0.6.0`. No longer maintained; Rust rewrite is the canonical version going forward.
- **v1.0+** — Rust. This codebase.

## Related

- [Surfpool](https://github.com/solana-foundation/surfpool) — the local Solana validator Tidepool runs on top of
- [Helius DAS](https://www.helius.dev/docs/api-reference/das) — the production API Tidepool mimics
- [Metaplex MplCore](https://developers.metaplex.com/core), [Bubblegum](https://developers.metaplex.com/bubblegum) — the asset standards
- [mpl-core](https://crates.io/crates/mpl-core), [mpl-token-metadata](https://crates.io/crates/mpl-token-metadata), [mpl-bubblegum](https://crates.io/crates/mpl-bubblegum) — the official Rust crates Tidepool uses

## Built by

[Tyler Buchea](https://github.com/TylerTheBuildor) — building on Solana since the Candy Machine era. Built Tidepool because every team I've worked with on Helius hit the same local-dev wall. [Twitter](https://twitter.com/TylerTheBuildor).

## License

MIT
