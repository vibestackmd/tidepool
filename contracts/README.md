# Contract testing

Tidepool's response shapes are validated in three layers:

1. **Structural** — the `tidepool_info` manifest (compat levels, since_version stamps, counts).
2. **Functional** — the ~230 unit + integration tests in `crates/` that drive handlers end-to-end with deterministic fixtures.
3. **Compatibility** (this directory) — fixtures recorded against real Helius, plus schemas derived from them, plus a drift-detection workflow that re-records and diffs.

Layer 3 is where we gain confidence that **our wire shapes actually match Helius's**, and that we'll notice when they change something.

## Layout

```
contracts/
  cases.toml                  # curated (method + params) list
  fixtures/<method>/<case>.json
                              # Phase 1: raw recorded responses,
                              # committed to git
  schemas/<method>/<case>.schema.json
                              # Phase 2: JSON Schema derived from each
                              # fixture (not yet implemented)
```

## Setup

**Local dev**: copy `.env.example` → `.env` (gitignored), fill in `HELIUS_API_KEY`. `cargo xtask ...` picks it up automatically via dotenvy.

**CI**: set `HELIUS_API_KEY` as a GitHub Actions repository secret. Referenced by `.github/workflows/helius-drift.yml` as `${{ secrets.HELIUS_API_KEY }}`.

## Workflows

### Record fixtures

```bash
cargo xtask record-helius
# or, for a single case:
cargo xtask record-helius --only getAsset_mad_lads_1337
```

Writes `contracts/fixtures/<method>/<case>.json` for every case in `cases.toml`. Skips on rate-limit / upstream errors with a warning; continues to the next case. Commit the results.

### Adding a new case

Edit `cases.toml` — add a new `[[case]]` block with a `name`, `method`, and `params`. Re-run the recorder. Rule of thumb: one happy-path case per method plus one or two edge cases that stress unusual shapes.

### Derive schemas

```bash
cargo xtask derive-schemas
```

Walks `fixtures/` offline, infers a Draft 7-ish shape schema per response, writes to `schemas/<method>/<case>.schema.json`. Commit alongside the fixture.

### Validate (every PR, offline)

`cargo test -p tidepool-rpc --test contracts` runs four tests:

1. **Schema self-validation** — every committed schema validates its own source fixture. Catches hand-edited divergence.
2. **Type deserialization** — `DasAsset` parses every item in the recorded `getAssetsByOwner` response. Fails if Helius added a required field we don't know about.
3. **Round-trip field coverage** — Rust type → JSON → diff against original fixture. Any field Helius returns that we silently drop fails the test with the missing key path.
4. **Priority-fee shape** — our `PriorityFeeLevels` type parses the recorded fee shape without loss.

These are offline — no network, no API key needed. They run on every PR.

### Drift detection (weekly, GitHub Actions)

`.github/workflows/helius-drift.yml` fires every Monday + on `workflow_dispatch`. It:

1. Re-records every fixture against real Helius.
2. Re-derives schemas.
3. Runs the offline contract tests against the refreshed artifacts.
4. If `contracts/` has any diff, opens a PR titled `"Helius contract drift (weekly)"` with a review checklist.

Maintainer's job: review the diff. Additions are usually safe to accept. Field renames or removals = real work: fix our Rust types before merging.

## Why fixtures rather than Pact / consumer-driven contracts

Pact assumes a cooperative provider who runs your expectations in their CI. Helius doesn't. What we actually want is **provider-fixture capture**:

- We own the canonical list of inputs (`cases.toml`).
- We keep a byte-for-byte record of what Helius returned (`fixtures/`).
- We compute the shape they imply (`schemas/`).
- We re-record on a schedule to catch drift.

The fixtures themselves are in version control, so every refactor that touches wire shapes can be reviewed against "what did Helius actually say" — not against our mental model.

## What this does NOT do

- **It doesn't replace the unit/integration tests.** Those cover logic; this covers wire fidelity. Both layers matter.
- **It doesn't gate PRs on network access.** The recorder is a manual / weekly run. Schema validation (Phase 2) runs off the committed fixtures and is fully offline.
- **It doesn't cover TypeScript.** The napi bridge passes through Rust; wire-shape guarantees live at the Rust layer. JS smoke tests stay in their "does the addon load" lane.
