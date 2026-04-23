# Rust library integration

Shows how a Rust consumer composes Tidepool's service layer directly — no HTTP, no CLI, just async function calls against the in-memory stores.

Useful when you're writing:

- Integration tests for a Rust backend that calls Helius.
- Custom tooling that needs DAS responses without spinning up a server.
- A consumer of `surfpool-sdk`'s Rust crate that wants DAS on top.

## Run

```bash
cargo run -p tidepool-example-rust-integration
```

Expected output:

```text
== getAsset ==
  id:         …
  interface:  V1_NFT
  owner:      …
  name:       Asset #1
  tree:       …
  leaf_id:    0
  asset_hash: …

== getAssetProof ==
  root:       …
  node_index: 256
  proof nodes: 8 entries

== tree inventory ==
  2 leaves indexed
```

## The public API

Four things you touch from user code:

```rust
use tidepool_rpc::cnft::{
    apply_event, CnftEvent, CnftStore, MemoryCnftStore, MintMetadata,
};
use tidepool_rpc::das::{get_asset, get_asset_proof};
```

That's the headline crate surface. Drop in a real `UpstreamClient` (we ship `HttpUpstream` in `tidepool-server`, or bring your own) to populate the store from a live RPC; drop in a custom `CnftStore` impl to back it with SQLite or whatever you want.
