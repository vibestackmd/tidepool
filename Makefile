# Tidepool — contributor operations.
#
# Maintainer-side Makefile. User-facing docs live in the README.
# Use `make help` to list every target.

.PHONY: install up down logs status \
        build test lint fmt check \
        dev run \
        node-build node-test \
        example-rust example-msw \
        version bump preflight release-dry-run tag-release push-release _require-clean \
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
	cargo run -p tidepool-cli -- start \
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
	cargo run -p tidepool-example-rust-integration

example-msw: node-build ## Run the MSW + vitest integration example
	cd examples/msw-integration && pnpm install --ignore-scripts && pnpm test

# ── Release ──────────────────────────────────────────────
# Local targets bump + preflight; publishing is done by CI after a
# signed tag push. See docs/release.md for the full pipeline.

version: ## Print the single-source workspace version
	@bash scripts/lib.sh; \
	python3 -c "import tomllib; print(tomllib.load(open('Cargo.toml','rb'))['workspace']['package']['version'])"

bump: _require-clean ## Bump version across workspace + npm (make bump V=1.0.0)
	@if [ -z "$(V)" ]; then echo "usage: make bump V=1.2.3"; exit 2; fi
	@bash scripts/bump-version.sh "$(V)"

preflight: ## Run the full release preflight (no release pinning)
	@bash scripts/preflight.sh

release-dry-run: _require-clean ## Run preflight for a target version (make release-dry-run V=1.0.0)
	@if [ -z "$(V)" ]; then echo "usage: make release-dry-run V=1.2.3"; exit 2; fi
	@bash scripts/preflight.sh --release "$(V)"

tag-release: _require-clean ## Create a signed tag for the current workspace version
	@V=$$( $(MAKE) -s version ); \
	if [ -z "$$V" ]; then echo "✖ workspace version unreadable"; exit 1; fi; \
	echo "Creating signed tag v$$V (requires GPG/SSH signing config)"; \
	git tag -s "v$$V" -m "Release v$$V"; \
	echo "Push with: git push origin v$$V"

_require-clean:
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "✖ Working tree not clean. Commit or stash changes first."; \
		exit 1; \
	fi

push-release: ## Push main + tags → CI release workflow fires on the tag
	git push origin HEAD
	git push origin --tags
	@echo ""
	@echo "  Tag pushed. Release workflow queued — confirm the"
	@echo "  'production' environment approval to actually publish."

# ── Cleanup ──────────────────────────────────────────────

clean: ## Remove all build artifacts
	cargo clean
	rm -rf crates/node/node_modules crates/node/*.node crates/node/index.js crates/node/index.d.ts
	rm -rf examples/*/node_modules

# ── Help ─────────────────────────────────────────────────

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

.DEFAULT_GOAL := help
