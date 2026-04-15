# surfpool-helius

**Helius DAS, running on your laptop.** A drop-in Helius-compatible RPC proxy for local Solana development. Point your app at it, ship to production Helius without changing a line of client code.

## Quickstart

You need [Surfpool](https://github.com/solana-foundation/surfpool) running locally, and Node 20+.

```bash
# 1. Start Surfpool (any of these work)
surfpool start                                            # native install
docker run --rm -p 8899:8899 -p 8900:8900 \
  surfpool/surfpool:latest start --no-tui --host=0.0.0.0  # or Docker

# 2. Run the proxy
npx surfpool-helius
```

Point your app at the proxy instead of Helius:

```ts
import { Connection } from "@solana/web3.js";

const connection = new Connection("http://localhost:8897", "confirmed");
```

Or for DAS calls:

```bash
curl -X POST http://localhost:8897 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getAsset","params":{"id":"<asset-address>"}}'
```

That's it. Everything you'd normally send to Helius — DAS queries, standard RPC, `confirmTransaction()` — now works locally.

## Why this exists

You're building on Solana with [Helius](https://www.helius.dev/) in production. Then you try to run locally and everything breaks:

- `solana-test-validator` and Surfpool don't speak DAS. No `getAsset`, no `searchAssets`.
- Hitting real Helius from dev costs money, needs internet, and pollutes prod traffic.
- Surfpool's WebSocket doesn't support `signatureSubscribe`, so `confirmTransaction()` hangs.

`surfpool-helius` sits between your app and Surfpool. It implements Helius DAS on real on-chain data, polyfills `signatureSubscribe` over HTTP polling, and passes everything else straight through. Same code path in dev and prod. No flags, no mocks.

## Architecture

```
  ┌─────────────┐        ┌──────────────────┐        ┌───────────┐
  │   Your app  │───────▶│  surfpool-helius │───────▶│  Surfpool │
  │  RPC_URL =  │        │  HTTP :8897      │        │   :8899   │
  │   :8897     │        │    WS :8898      │        │   WS :8900│
  └─────────────┘        │                  │        └───────────┘
                         │  DAS:            │
                         │  • getAsset      │              ▲
                         │  • searchAssets  │              │
                         │                  │              │
                         │  WS polyfill:    │              │
                         │  • sigSubscribe  │──────────────┘
                         │                  │   (HTTP polling)
                         │  Everything else │
                         │  → passthrough   │
                         └──────────────────┘
```

`getAsset` fetches the raw account from Surfpool via `getAccountInfo`, runs it through a pluggable decoder (MplCore by default), and returns a DAS-shaped response. Every asset that passes through the proxy is indexed by owner, authority, and grouping, so `searchAssets`, `getAssetsByOwner`, `getAssetsByGroup`, and `getAssetsByAuthority` all serve from that index — no `getProgramAccounts` scan, which would never terminate against Surfpool's mainnet-forked upstream anyway. Assets the proxy has never fetched are invisible to the index, which is the documented local-dev tradeoff.

## Supported methods

| Method                  | Status  | Notes                                                             |
|-------------------------|---------|-------------------------------------------------------------------|
| `getAsset`              | ✅ v0.1 | MplCore assets + collections                                      |
| `getAssetBatch`         | ✅ v0.2 | Up to 1000 ids per batch; parallel upstream reads                 |
| `getAssetsByOwner`      | ✅ v0.2 | Indexes assets as they're fetched                                 |
| `getAssetsByGroup`      | ✅ v0.2 | For MplCore: `groupKey: "collection"`                             |
| `getAssetsByAuthority`  | ✅ v0.2 | Matches the MplCore update authority                              |
| `searchAssets`          | ✅ v0.2 | Full filter surface: owner, authority, grouping, interface, type  |
| `signatureSubscribe`    | ✅ v0.1 | Polyfilled via HTTP polling                                       |
| `signatureUnsubscribe`  | ✅ v0.1 | Cancels the polling timer                                         |
| `surfpoolHeliusInfo`    | ✅ v0.1 | Custom. Returns the full compatibility manifest for introspection |
| `getAssetsByCreator`    | ⏳ v0.3 | Needs MplCore plugin parsing or Token Metadata decoder            |
| `getAssetProof`         | ⏳ v0.3 | Compressed NFT merkle proofs                                      |
| `getNftEditions`        | ⏳ v0.3 | Needs Token Metadata decoder                                      |
| Everything else         | ✅ v0.1 | Passed through to Surfpool unchanged                              |

**Tip:** POST `{"method": "surfpoolHeliusInfo"}` to the proxy to get a live, machine-readable list of every Helius method and its local compatibility level. That's the source of truth — this table is regenerated from it.

## Configuration

All options are optional; defaults target a standard local Surfpool install.

**CLI:**

```bash
npx surfpool-helius \
  --port 8897 \
  --upstream http://127.0.0.1:8899 \
  --upstream-ws-port 8900
```

**Environment variables:** `SURFPOOL_HELIUS_PORT`, `SURFPOOL_HELIUS_UPSTREAM_URL`, `SURFPOOL_HELIUS_UPSTREAM_WS_PORT`.

**Programmatic:**

```ts
import { createProxy, mplCoreDecoder } from "surfpool-helius";

await createProxy({
  port: 8897,                            // HTTP; WS binds to port + 1
  upstreamUrl: "http://127.0.0.1:8899",
  upstreamWsPort: 8900,
  rpcTimeoutMs: 10_000,
  decoders: [mplCoreDecoder],
});
```

## Pluggable decoders

The built-in MplCore decoder handles `AssetV1` and `CollectionV1`. To support another program, implement `AccountDecoder` and pass it in:

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

The proxy picks a decoder by matching the account's owner program ID. First match wins. Pass `decoders: []` to disable DAS entirely and run as a pure passthrough with the `signatureSubscribe` polyfill.

## FAQ

**Is this production-ready?** No. It's a local development tool. Ship to real Helius in production.

**Does this replace Helius?** No. It lets you develop against Helius locally so your production integration has a tight dev loop.

**Is this endorsed by Helius?** No — it's a community tool. Helius is great and you should use them.

**Why not depend on `@metaplex-foundation/mpl-core`?** That package is UMI-flavored and pulls in a large tree. For a lean proxy, we hand-roll the minimum Borsh deserialization needed to populate a DAS response. The `AccountDecoder` interface is pluggable — drop in Codama-generated decoders or the UMI package if you prefer.

**Does this work with compressed NFTs?** Not yet — `getAssetProof` and merkle tree reads are on the v0.3 roadmap.

**Can I use this with `litesvm` or `solana-test-validator` instead of Surfpool?** Yes. The proxy just needs something that speaks standard Solana RPC. Point `--upstream` at any RPC endpoint. Surfpool is the default because it also gives you mainnet forking.

**Does this work with Solana RPC 2.0 / Triton's new read layer?** Yes. surfpool-helius proxies standard JSON-RPC, which [RPC 2.0](https://blog.triton.one/announcing-rpc-2-0-with-solana-foundation-rethinking-solanas-read-layer-from-the-ground-up/) preserves for backward compatibility. DAS endpoints aren't part of RPC 2.0, so the local dev gap this tool fills stays relevant.

## Example

[`examples/mint-and-query`](examples/mint-and-query) is a full end-to-end proof: it points UMI at the proxy, mints an MplCore asset, waits for confirmation (which exercises the `signatureSubscribe` polyfill), then fetches it back via `getAsset`. If you want to see every moving part exercised, run that.

## Roadmap

- **v0.1** — MplCore decoder, `getAsset`, `searchAssets`, `signatureSubscribe` polyfill, pass-through proxy, compatibility manifest
- **v0.2** — `getAssetBatch`, `getAssetsByOwner`, `getAssetsByGroup`, `getAssetsByAuthority`, full `searchAssets` filtering, `authorities` field on decoded assets
- **v0.3** — MplCore plugin parsing (unlocks `getAssetsByCreator`), priority fee estimation, Helius V2 RPC wrappers, staking helpers
- **v0.4** — Compressed NFT support (`getAssetProof`), Enhanced Transactions parser for common tx types, Token Metadata decoder
- **v0.5** — Local webhook simulator (polling-based delivery against Surfpool)
- **Maybe** — Rust port, standalone binary, local [Dragon's Mouth](https://docs.triton.one/project-yellowstone/dragons-mouth-grpc-subscriptions) (Yellowstone gRPC) polyfill

## Related

- [Surfpool](https://github.com/solana-foundation/surfpool) — the local Solana validator this runs on top of
- [Helius DAS API](https://www.helius.dev/docs/api-reference/das) — the production API this mimics
- [Metaplex MplCore](https://developers.metaplex.com/core) — the NFT standard the default decoder handles
- [Triton RPC 2.0](https://blog.triton.one/announcing-rpc-2-0-with-solana-foundation-rethinking-solanas-read-layer-from-the-ground-up/) — where Solana's read layer is headed

## Built by

[Tyler Buchea](https://github.com/TylerTheBuildor) — I've been building on Solana since the Candy Machine era. I built this because I needed it for a project I'm working on, and every team on Helius has felt the same pain. [Twitter](https://twitter.com/TylerTheBuildor).

## License

MIT
