# Tidepool — contributor operations.
#
# Maintainer-side Makefile. User-facing docs live in the README.
# Use `make help` to list every target.

.PHONY: install up down logs status \
        build test lint fmt check \
        dev run \
        node-build node-test \
        example-rust example-msw \
        version release-patch release-minor release-major push-release \
        clean help

# ── Setup ────────────────────────────────────────────────

install: ## Install Rust deps + pnpm deps for examples and napi bridge
	cargo fetch
	cd crates/node && pnpm install --ignore-scripts
	cd examples/msw-integration && pnpm install --ignore-scripts

# ── Infrastructure ───────────────────────────────────────

up: ## Start Surfpool in Docker (adjacent to Tidepool)
	docker compose up -d
	@echo ""
	@echo "  Surfpool:        http://localhost:8899"
	@echo "  Surfpool WS:     ws://localhost:8900"
	@echo "  Surfpool Studio: http://localhost:8488"
	@echo ""
	@echo "  Start Tidepool:  make dev"

down: ## Stop Surfpool
	docker compose down

logs: ## Tail Surfpool logs
	docker compose logs -f surfpool

status: ## Show Surfpool container status
	docker compose ps

# ── Rust workspace ───────────────────────────────────────

build: ## cargo build --release across the workspace
	cargo build --release --workspace

test: ## cargo test --workspace
	cargo test --workspace

lint: ## clippy across the workspace under pedantic lints
	cargo clippy --workspace --all-targets -- -D warnings

fmt: ## rustfmt everything
	cargo fmt --all

check: lint test ## Lint + test (local pre-push gate)

dev: ## Run the CLI against a local Surfpool
	cargo run -p tidepool-rpc-cli -- start \
		--port 8897 \
		--upstream http://127.0.0.1:8899

run: dev ## Alias for `dev`

# ── Node bridge ──────────────────────────────────────────

node-build: ## Build the napi .node addon (release mode)
	cd crates/node && pnpm run build

node-test: node-build ## Run the JS smoke tests against the built addon
	cd crates/node && node --test __test__

# ── Examples ─────────────────────────────────────────────

example-rust: ## Run the Rust library integration example
	cargo run -p tidepool-rpc-example-rust-integration

example-msw: node-build ## Run the MSW + vitest integration example
	cd examples/msw-integration && pnpm install --ignore-scripts && pnpm test

# ── Release ──────────────────────────────────────────────
# Local targets bump, commit, and tag — they never publish.
# Publishing is deliberately a manual GitHub Actions dispatch;
# see .github/workflows/node-prebuild.yml (dormant until a
# maintainer enables it).

version: ## Print current workspace version
	@grep -m1 '^version' crates/cli/Cargo.toml | sed 's/version = "\(.*\)"/\1/'

release-patch: _require-clean ## Bump patch version across the workspace + tag
	@$(MAKE) _bump BUMP=patch

release-minor: _require-clean ## Bump minor version across the workspace + tag
	@$(MAKE) _bump BUMP=minor

release-major: _require-clean ## Bump major version across the workspace + tag
	@$(MAKE) _bump BUMP=major

_require-clean:
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "✖ Working tree not clean. Commit or stash changes first."; \
		exit 1; \
	fi

_bump:
	@echo "Bumping ${BUMP} across crates/*/Cargo.toml + crates/node/package.json"
	@# Delegate to cargo-release if available, else fall back to a manual bump
	@if command -v cargo-release >/dev/null; then \
		cargo release $(BUMP) --workspace --no-publish --no-push --execute; \
	else \
		echo "cargo-release not installed. Install via: cargo install cargo-release"; \
		exit 1; \
	fi

push-release: ## Push release commit + tags → CI can then publish
	git push origin HEAD
	git push origin --tags
	@echo ""
	@echo "  Pushed. Publishing is gated on manual workflow_dispatch."

# ── Cleanup ──────────────────────────────────────────────

clean: ## Remove all build artifacts
	cargo clean
	rm -rf crates/node/node_modules crates/node/*.node crates/node/index.js crates/node/index.d.ts
	rm -rf examples/*/node_modules

# ── Help ─────────────────────────────────────────────────

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

.DEFAULT_GOAL := help
