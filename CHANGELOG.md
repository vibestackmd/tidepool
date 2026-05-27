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

### Changed
- **WS server is now a thin reverse proxy** instead of an HTTP-polling
  polyfill. Surfpool (v1.1+) natively implements `signatureSubscribe`,
  `accountSubscribe`, `logsSubscribe` (incl. the `mentions` filter we
  used to polyfill), `programSubscribe`, and `slotSubscribe`. Tidepool
  now opens one WS connection to Surfpool's WS port (default 8900) per
  client and pumps frames both directions. ~1,150 lines deleted; ~180
  lines added. Single-endpoint UX preserved (clients still hit
  `ws://tidepool:port+1`).
- Three WS manifest entries (`signatureSubscribe`, `accountSubscribe`,
  `logsSubscribe`) move from compat level `SHIM` → `EXACT`.

### Fixed
- CI: `node --test __test__` failed on Node 24 — the bare directory
  is interpreted as a module path. Switched to an explicit glob
  (`__test__/*.test.mjs`).
- CI: `verify-changelog.sh --expected unreleased` now treats a
  release commit at HEAD as a legitimate empty `[Unreleased]`. The
  bullet requirement applies to PRs and in-flight main commits,
  not to the immediate post-release state.

### Docs
- README polish: live crates.io + npm version badges, fixed
  `tidepool-rpc = "0.1"` (was "1"), Roadmap/Versions now match
  shipped 0.1.x reality.
- Launch announcement at `announcements/v0.1.5.md`.
- Release workflow header rewritten to capture lessons from the
  v0.1.1–v0.1.5 release iteration (Node 24 OIDC requirement, no
  npm pending publisher, dtolnay/rust-toolchain pinning, native
  ARM Linux runner, fat-package npm layout).

## [0.1.5] — 2026-05-26

### Fixed
- Release CI: bumped GitHub Actions Node version from 22 to 24 in
  `actions/setup-node` steps. Node 22 ships with npm 10, which
  doesn't support the latest OIDC handshake; the registry rejects
  the publish with a misleading `404 "is not in this registry"`
  *after* sigstore signs the provenance attestation. Node 24
  brings npm 11.5+, which actually completes the OIDC exchange.
  This is the documented npm trusted-publishing gotcha.

## [0.1.4] — 2026-05-26

Fixed the npm-publish step end-to-end.

### Fixed
- Release CI: switched to "fat package" layout — all platform `.node`
  files ship in the main `@vibestackmd/tidepool` package; the napi
  loader picks the right one at require-time. Removes the dance
  around publishing five per-platform sub-packages (each of which
  would need its own trusted-publisher config).
- Release CI: dropped `napi artifacts` step, which was assembling
  the multi-package layout we no longer want. Replaced with a plain
  `cp artifacts/*.node .` into the package root before publish.
- `crates/node/index.js` + `crates/node/index.d.ts` are now
  committed (no longer gitignored). They're auto-generated but only
  change when napi config changes, so CI doesn't need a Rust
  toolchain in the publish-npm step to regenerate them. Also fixes
  an earlier stale-name issue (the previously-generated `index.js`
  referenced `tidepool-rpc-*` sibling packages from the pre-rename
  era).

## [0.1.3] — 2026-05-26

First truly-lockstep release: crates + multi-platform npm both ship.

### Fixed
- Release CI: `actions/download-artifact@v4` now uses
  `merge-multiple: true`. Without it, each `bindings-<target>`
  artifact landed in its own subdirectory and `napi artifacts`
  couldn't find the `.node` files at the path it expects.

## [0.1.2] — 2026-05-26

First lockstep release across both registries. Crates and npm
package all at 0.1.2; npm package finally ships with multi-platform
prebuilds (darwin x64/arm64, linux x64/arm64, windows x64).

### Fixed
- Release CI: pinned `dtolnay/rust-toolchain` action to 1.94.1 to
  match `rust-toolchain.toml`. Floating `@stable` installed 1.95.0
  with cross-compile targets, but cargo respected the pin and ran
  1.94.1 without the target — broke darwin x86_64 prebuild.
- Release CI: switched the linux ARM64 prebuild to the native
  `ubuntu-24.04-arm` runner. Cross-compiling from x86_64 ubuntu
  was missing `gcc-aarch64-linux-gnu`; native runner sidesteps
  the C-toolchain install.

## [0.1.1] — 2026-05-26

First release published via the OIDC CI pipeline. End-to-end exercise
of trusted publishing on crates.io and npm; first npm release with
multi-platform prebuilds (darwin x64/arm64, linux x64/arm64, windows
x64). v0.1.0 of `@vibestackmd/tidepool` was skipped — `0.0.1` was
published locally as a deprecated placeholder to enable the npm
trusted-publisher form. The crates and the npm package are now in
lockstep at 0.1.1.

### Fixed
- Release preflight: compare against the *prior* `release: v*` commit,
  not HEAD itself (the old logic gave a trivially-empty diff in CI's
  tag checkout and locally pre-tag).
- Release preflight: missing signing key is now a warning, not an
  error — CI's `git verify-tag` step remains the hard gate.
- Release preflight: first-publish dry-run skips dependent crates
  (their workspace siblings aren't on crates.io until the first
  publish completes).
- `tidepool-rpc`: `crates/service/compatibility.toml` symlinks the
  workspace-root file so `cargo publish` bundles it in the package
  tarball.
- Release workflow smoke test: corrected stale `tidepool-rpc`
  references to the actual CLI binary (`tidepool`) and npm package
  (`@vibestackmd/tidepool`).

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
