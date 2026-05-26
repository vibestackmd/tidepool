# Changelog

All notable changes to Tidepool are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Every release covers all five `tidepool-*` crates on crates.io and
the `@vibestackmd/tidepool` npm package simultaneously — lockstep versioning so
there's no "which version is compatible with which?" drift. Upstream
pins for each release are in `compatibility.toml`.

The release pipeline (`scripts/preflight.sh` + `.github/workflows/release.yml`)
refuses to publish a version that doesn't have an entry here.

## [Unreleased]

## [0.1.0] — 2026-05-26

First public release of the Rust rewrite. Five `tidepool-*` crates publish
to crates.io in lockstep; `@vibestackmd/tidepool` ships the napi bridge to
npm. `tidepool-cli` is the supported entry point; the other crates are
internal until 1.0.

### Added
- Single-source versioning across the workspace — `[workspace.package].version`
  is inherited by every `tidepool-*` crate.
- `compatibility.toml` documents the exact Surfpool / helius-sdk / Solana /
  Rust / Node versions this release was tested against. Surfaced via
  `tidepool_info.result.compatibility` and inline in the CLI's `--version`
  output.
- `xtask check-drift` subcommand for the drift-detection workflow.
- `xtask record-helius --transport` filter; `xtask derive-schemas --only`
  filter.
- REST transport layer on `/v0/…` paths (getBalances, getTransactions,
  getTransactionsByAddress, full webhook CRUD). Mirrors Helius's
  JSON-RPC vs. REST split exactly.
- `logsSubscribe` WS polyfill (mentions filter; `all` / `allWithVotes`
  rejected with typed -32601).
- Enhanced-tx `tokenStandard` enrichment from the DAS cache.
- Contract-test rig (phase 1-3): fixture recorder, schema derivation,
  offline round-trip tests. Caught 10+ real drift bugs this cycle.
- cNFT contract fixtures (`getAsset`, `getAssetProof`).
- SQLite backend concurrency smoke test.

### Changed
- Token Metadata owner resolution applies to every decoder-empty asset,
  not just `interface == "V1_NFT"` — fixes pNFT owner regression.
- `EnhancedTransaction` always serializes `accountData`, `lighthouseData`,
  `transactionError`; `accountData` populated from pre/postBalances.
- `DasAsset.last_indexed_slot` added (cNFT only; skip-on-None).
- `DasCompression` gains Bubblegum V2 fields (`collection_hash`,
  `asset_data_hash`, `flags`).
- `DasOwnership.non_transferable` added.
- `DasSupply` fields emit unconditionally (match Helius null-on-empty).

### Fixed
- Integration-test port race eliminated via `pick_two_free_ports()`.

<!--
Entry template for future releases:

## [x.y.z] — YYYY-MM-DD

### Added
- ...

### Changed
- ...

### Deprecated
- ...

### Removed
- ...

### Fixed
- ...

### Security
- ...

### Compatibility
- Surfpool: tested range updated to ...
- helius-sdk: ...
-->
