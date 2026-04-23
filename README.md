<div align="center">

<img src="./assets/logo.png" alt="Tidepool logo" width="180" />

# Tidepool

**Helius DAS, locally. Built on Surfpool.**

A Helius-compatible local dev environment for Solana — DAS, compressed NFTs, WebSocket subscriptions, Enhanced Transactions, and webhooks, all from a single Rust binary you run next to your validator. Your production `helius-sdk` integration works offline, without a key, without cost.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Rust 2021](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org)
[![MSRV 1.77](https://img.shields.io/badge/MSRV-1.77-orange.svg)](./Cargo.toml)
[![CI](https://github.com/TylerTheBuildor/tidepool/actions/workflows/ci.yml/badge.svg)](https://github.com/TylerTheBuildor/tidepool/actions/workflows/ci.yml)

</div>

---

## Why

Three things you'll notice in the first five minutes.

**🌊 &nbsp; Local DAS, including compressed NFTs.** &nbsp; `getAsset`, `getAssetBatch`, `getAssetProof`, the `getAssetsBy*` family, `searchAssets`. MplCore, Token Metadata (both Token and Token-2022), and Bubblegum cNFTs — all resolved locally from real on-chain bytes. cNFTs go through a full Bubblegum indexer that replays every tree-mutating instruction; authoritative state comes from the noop-CPI `LeafSchemaEvent`, so proofs match on-chain even after a `setAndVerifyCollection`.

**⚡ &nbsp; `confirmTransaction()` actually works on Surfpool.** &nbsp; Surfpool's native WebSocket doesn't implement `signatureSubscribe`, so `@solana/web3.js`'s `confirmTransaction()` hangs. Tidepool polyfills it. Every `helius-sdk` method that composes "send, wait, assert" — `sendSmartTransaction`, `broadcastTransaction`, `pollTransactionConfirmation` — just works.

**🧪 &nbsp; Plugs into MSW / Nock / undici for tests.** &nbsp; Import `tidepool-rpc` from npm, plug `handleJsonRpcBody` into whichever mock-HTTP layer your team uses. Your test suite gets deterministic Helius responses without standing up a validator.

---

## Quickstart

Three ways to consume it. Pick one.

### As a binary

```bash
# Terminal 1
surfpool start

# Terminal 2
cargo install tidepool-rpc-cli   # or `npx tidepool-rpc start` post-1.0
tidepool-rpc start \
  --port 8897 \
  --upstream http://127.0.0.1:8899 \
  --index-tree <your-cNFT-merkle-tree>
```

```ts
// Your app
import { Connection } from "@solana/web3.js";
const connection = new Connection("http://localhost:8897", "confirmed");
```

### As a Rust library

```toml
# Cargo.toml
[dependencies]
tidepool-rpc = "1"
```

```rust
use tidepool_rpc::cnft::{apply_event, CnftEvent, MemoryCnftStore};
use tidepool_rpc::das::{get_asset, get_asset_proof};
```

Full example in [`examples/rust-integration/`](examples/rust-integration/).

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

Point the same client at Tidepool — one URL swap, every transport works. JSON-RPC, REST (`/v0/…`), and WebSocket all resolve against Tidepool because we mirror Helius's transport split exactly.

```ts
import { Helius } from "helius-sdk";

const helius = new Helius(
  process.env.HELIUS_API_KEY ?? "local-dev",
  process.env.NODE_ENV === "development"
    ? { url: "http://localhost:8897", restUrl: "http://localhost:8897" }
    : undefined, // prod: default to mainnet.helius-rpc.com + api.helius.xyz
);

await helius.rpc.getAsset({ id: mintPubkey });             // JSON-RPC
await helius.enhanced.getTransactions([signature]);        // REST
await helius.ws.signatureNotifications(signature);         // WS polyfill
```

The `restUrl` + `url` split assumes a small PR landing in [`helius-labs/helius-sdk`](https://github.com/helius-labs/helius-sdk) to make the REST base URL configurable. Until it merges, JSON-RPC + WS work today; REST needs the SDK's internal base URL overridden via whatever escape hatch your SDK version provides.

---

## How it works

Tidepool sits between your app and Surfpool. Requests for methods we own (DAS, cNFT proofs, enhanced tx, webhooks, WS polyfills) are served from local state; everything else is forwarded to Surfpool unchanged.

- **Uncompressed `getAsset`** fetches the account from the upstream, runs it through a pluggable decoder (`mpl-core` / `mpl-token-metadata`), returns a DAS-shaped response. The cache populates as a side effect so `searchAssets`, `getAssetsByOwner`, and the other secondary-index queries work immediately.
- **Compressed `getAsset` / `getAssetProof`** resolve from a local Bubblegum indexer: `getSignaturesForAddress` walks the tree, `getTransaction` pulls each candidate tx, inner Bubblegum + noop CPIs are parsed for authoritative leaf state. Trees are registered via `--index-tree` at startup or `tidepool_indexTree` at runtime.
- **Everything unknown** falls through to the upstream unchanged. Standard Solana RPC (`getSlot`, `sendTransaction`, `getProgramAccounts`, etc.) works with zero code on our side.

### Why Surfpool as the upstream?

Tidepool works with any standard Solana RPC — `solana-test-validator` with `--clone`, real devnet, a self-hosted node. **Surfpool is recommended** because its mainnet-forking means any real account you ask about "just works" without pre-declaring it. That's what makes the dev-loop feel magic instead of tedious. The `signatureSubscribe` polyfill specifically exists because Surfpool doesn't implement it, so Tidepool delivers strictly more value here than against anything else.

---

## Supported methods

Full live truth: `POST {"method":"tidepool_info"}` returns the complete manifest. Every entry is classified `EXACT`, `LOCAL_INDEX`, `BEST_EFFORT`, `SHIM`, `SDK_WRAPPER`, `PLANNED`, or `SKIPPED`.

| Method | Status | Notes |
|---|---|---|
| `getAsset` / `getAssetBatch` | ✅ LOCAL_INDEX | MplCore, Token Metadata (incl. Token-2022), cNFTs |
| `getAssetProof` / `getAssetProofBatch` | ✅ LOCAL_INDEX | Requires tree registered via `--index-tree` or runtime method |
| `getAssetsByOwner` / `Authority` / `Creator` / `Group` | ✅ LOCAL_INDEX | Cache-backed secondary indexes |
| `searchAssets` | ✅ LOCAL_INDEX | Multi-filter AND, smallest-index-first narrowing |
| `getNftEditions` | ✅ LOCAL_INDEX | Lazy edition-PDA indexing; master + print editions |
| `signatureSubscribe` / `accountSubscribe` / `logsSubscribe` (+ `Unsubscribe`) | ✅ SHIM | HTTP polling polyfills on the WS port. `logsSubscribe` supports `{ mentions: [pubkey] }` only. |
| `getPriorityFeeEstimate` | ✅ BEST_EFFORT | Local percentile ladder over `getRecentPrioritizationFees` |
| `helius-sdk` composed methods | ✅ SDK_WRAPPER | Send / broadcast / confirm / staking — all work transparently |
| `getBalances` (REST) | ✅ SHIM | `GET /v0/addresses/{addr}/balances` |
| `getTransactions` / `getTransactionsByAddress` (REST) | ✅ SHIM | Enhanced Transactions parsers on `/v0/transactions` and `/v0/addresses/{addr}/transactions` |
| `createWebhook` family (REST) | ✅ SHIM | Polling-simulator on `/v0/webhooks` + `/v0/webhooks/{id}` — full CRUD |
| Everything else | ✅ Passthrough | Forwarded to the upstream unchanged |

### Transports

Tidepool matches Helius's transport split exactly — a method lives where Helius puts it and nowhere else, so you can't write local-dev code that'd fail against production.

| Transport | Where | Methods |
|---|---|---|
| JSON-RPC | `POST /` | DAS (`getAsset*`), Bubblegum control (`tidepool_*`), standard RPC passthrough |
| REST | `/v0/…` | Wallet (`getBalances`), Enhanced Transactions, Webhooks CRUD |
| WebSocket | `ws://host:port+1` | `signatureSubscribe`, `accountSubscribe`, `logsSubscribe`, `*Unsubscribe` |
| SDK Wrapper | n/a | `sendSmartTransaction`, `broadcastTransaction`, `pollTransactionConfirmation`, stake/unstake |

`tidepool_info` returns a `transport` field per method so tooling can introspect without guessing.

---

## Compressed NFTs

cNFTs live as leaves in a Bubblegum merkle tree, not as standalone accounts. Tidepool ships a local indexer that replays every tree-mutating instruction into an in-memory (or SQLite-backed) store, from which `getAsset` / `getAssetProof` serve directly.

Register a tree:

```bash
# At startup
tidepool-rpc start --index-tree <merkle-tree>

# Or at runtime (in a vitest setup file, CI script, etc.)
curl -X POST http://localhost:8897 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tidepool_indexTree","params":{"tree":"<merkle-tree>"}}'
```

**Tracked instructions:** `createTree`, `mintV1` / `mintV2`, `mintToCollectionV1`, `transfer` / `transferV2`, `burn` / `burnV2`, `delegate` / `delegateV2`, `verifyCreator` / `verifyCreatorV2`, `unverifyCreator`, `verifyCollection`, `unverifyCollection`, `setAndVerifyCollection` / `setCollectionV2`, `updateMetadata` / `updateMetadataV2`. For hash-dependent ixs, authoritative state comes from the noop `LeafSchemaEvent` CPI — proofs stay correct through multi-step flows. Covers both SPL-NOOP (V1) and MPL-NOOP (V2) noop programs.

---

## Persistence

Default is **in-memory** — state is lost on restart. Two flags turn that off, shaped after [Surfpool's own persistence UX](https://github.com/txtx/surfpool) so the two tools feel familiar.

```bash
# Single SQLite file holds cNFT index + DAS cache + webhook registry
tidepool-rpc start --db ./tidepool.sqlite

# Preload snapshot(s) at boot; repeatable
tidepool-rpc start --snapshot ./trees/foo.json --snapshot ./trees/bar.json
```

`tidepool_exportTreeSnapshot` dumps a tree's indexed state at runtime; commit it to your repo and every fresh boot can `--snapshot` that file to skip re-paging tx history.

---

## Examples

- [`examples/msw-integration/`](examples/msw-integration/) — vitest + MSW + Tidepool, three runnable tests
- [`examples/rust-integration/`](examples/rust-integration/) — cargo example composing the service layer directly
- [`examples/README.md`](examples/) — all consumer patterns indexed

## Workspace layout

| Crate | Purpose |
|---|---|
| `tidepool-rpc-core` | Pure primitives: keccak, merkle math, LeafSchemaV1 hashing, proof compute/verify. Zero Solana deps — WASM-ready. |
| `tidepool-rpc` | Service layer: cNFT state machine, DAS handlers, cache, decoders, upstream trait. |
| `tidepool-rpc-server` | axum HTTP + WS front-end. Method-enum dispatch. `HttpUpstream` via reqwest. |
| `tidepool-rpc-cli` | `tidepool-rpc` binary. clap-derive args + env-var overlay. |
| `tidepool-rpc-node` | napi-rs bridge → the `tidepool-rpc` npm package. |

Library consumers pull `tidepool-rpc`. Binary users `cargo install tidepool-rpc-cli`. JS users `npm install tidepool-rpc`. Server builders compose `tidepool-rpc` + `tidepool-rpc-server::HttpUpstream` themselves.

---

## FAQ

<details>
<summary><b>Is this production-ready?</b></summary>

No. It's a local development tool. Ship to real Helius in production.
</details>

<details>
<summary><b>Does this replace Helius?</b></summary>

No. It lets you develop against Helius's API locally so your production integration has a tight dev loop.
</details>

<details>
<summary><b>Is this endorsed by Helius or Surfpool?</b></summary>

Community tool, no official endorsement. Both are great companies and you should use them.
</details>

<details>
<summary><b>Why not just hit real Helius in dev?</b></summary>

You'd burn rate limits, pollute prod monitoring, require internet on CI, and can't test without an API key. Tidepool is the answer to "I want the dev loop to be instant + offline."
</details>

<details>
<summary><b>Can I use this with `solana-test-validator` or `litesvm`?</b></summary>

`solana-test-validator` works — point `--upstream` at it, clone mainnet accounts via `--clone`. `litesvm` is in-process-only, so there's no RPC endpoint for Tidepool to proxy. Use Surfpool for the magic, test-validator for the boring-but-predictable case.
</details>

<details>
<summary><b>Why Rust?</b></summary>

The previous version was TypeScript (v0.6, preserved at that tag). The Rust rewrite earned: drop-in official `mpl-core` / `mpl-token-metadata` / `mpl-bubblegum` crates (no Codama pipeline), exhaustive-match method dispatch (compile-time safety for adding new handlers), type-level noop-required-vs-optional enforcement on cNFT events, binary distribution via `cargo install`. The napi-rs bridge means JS consumers still get the test-integration story via `npm install tidepool-rpc` — one Rust core, two consumption ecosystems.
</details>

<details>
<summary><b>Does the WS polyfill work over compressed transactions?</b></summary>

The polyfill polls `getSignatureStatuses`, which resolves any signature the validator has seen. Works for compressed + uncompressed transactions identically.
</details>

---

## Roadmap

- ✅ **v1.0** — Rust rewrite with MplCore / Token Metadata / cNFT decoders, full DAS surface, WS polyfills (`signatureSubscribe`, `accountSubscribe`, `logsSubscribe`), axum server, CLI binary, napi bridge, REST transport, webhooks CRUD, Enhanced Transactions
- **v1.1** — Token Metadata owner resolution for all interfaces, `tokenStandard` enrichment on enhanced tx, richer `accountData.tokenBalanceChanges`
- **v1.2** — USD pricing pass-through (once we have a curated source), additional WS subscriptions (`programSubscribe`, `slotSubscribe`)
- **Maybe** — Dragon's Mouth (Yellowstone gRPC) polyfill

## Versions

- **v0.1–v0.6** — TypeScript implementation. Preserved at tags `v0.1.0` through `v0.6.0`. No longer maintained.
- **v1.0+** — Rust. This codebase.

## Related

- [Surfpool](https://github.com/txtx/surfpool) — the local Solana validator Tidepool runs on top of
- [Helius DAS](https://www.helius.dev/docs/api-reference/das) — the production API Tidepool mimics
- [Metaplex MplCore](https://developers.metaplex.com/core), [Bubblegum](https://developers.metaplex.com/bubblegum) — the asset standards
- [mpl-core](https://crates.io/crates/mpl-core), [mpl-token-metadata](https://crates.io/crates/mpl-token-metadata), [mpl-bubblegum](https://crates.io/crates/mpl-bubblegum) — the official Rust crates Tidepool uses

---

<div align="center">

**MIT** · [LICENSE](./LICENSE)

</div>
