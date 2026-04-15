# surfpool-helius — contributor operations.
#
# This Makefile is for the maintainer. End-user docs live in the README.
# Use `make help` for the full list.

.PHONY: install up down logs status dev build typecheck clean \
        codegen codegen-mpl-core codegen-token-metadata \
        update-idl update-idl-mpl-core update-idl-token-metadata \
        version release-patch release-minor release-major \
        push-release help

# ── Setup ────────────────────────────────────────────────

install: ## Install dependencies
	pnpm install

# ── Infrastructure ───────────────────────────────────────

up: ## Start Surfpool in Docker
	docker compose up -d
	@echo ""
	@echo "  Surfpool:        http://localhost:8899"
	@echo "  Surfpool WS:     ws://localhost:8900"
	@echo "  Surfpool Studio: http://localhost:8488"
	@echo ""
	@echo "  Start the proxy with:  make dev"

down: ## Stop Surfpool
	docker compose down

logs: ## Tail Surfpool logs
	docker compose logs -f surfpool

status: ## Show Surfpool container status
	docker compose ps

# ── Proxy ────────────────────────────────────────────────

dev: ## Run the proxy in watch mode
	pnpm dev

build: ## Build TypeScript to dist/
	pnpm build

typecheck: ## Run TypeScript type checking
	pnpm typecheck

# ── Code generation ──────────────────────────────────────
# Each supported program is produced from a pinned IDL under idls/. The
# upstream source + commit SHA live in idls/<name>.source.json. Regen is
# a deliberate manual action — either `make codegen-<name>` (same pinned
# IDL) or `make update-idl-<name>` (fetch latest main, then regen).
#
# `make codegen` and `make update-idl` without a suffix are back-compat
# aliases for the mpl-core targets.

codegen: codegen-mpl-core ## Regenerate src/generated/mpl-core from the pinned IDL (alias)

codegen-mpl-core: ## Regenerate src/generated/mpl-core from the pinned IDL
	pnpm tsx scripts/codama.ts mpl-core

codegen-token-metadata: ## Regenerate src/generated/token-metadata from the pinned IDL
	pnpm tsx scripts/codama.ts token-metadata

update-idl: update-idl-mpl-core ## Fetch latest mpl-core IDL from main + regenerate (alias)

update-idl-mpl-core: ## Fetch latest mpl-core IDL from main + regenerate
	@$(MAKE) _update-idl REPO=metaplex-foundation/mpl-core IDL=mpl_core TARGET=mpl-core

update-idl-token-metadata: ## Fetch latest token-metadata IDL from main + regenerate
	@$(MAKE) _update-idl REPO=metaplex-foundation/mpl-token-metadata IDL=token_metadata TARGET=token-metadata

# Private helper: parameterized IDL refresh.
# Required vars: REPO, IDL, TARGET.
_update-idl:
	@echo "Fetching latest $(REPO) main commit..."
	@SHA=$$(gh api repos/$(REPO)/commits/main --jq '.sha'); \
	echo "  commit: $$SHA"; \
	curl -sL "https://raw.githubusercontent.com/$(REPO)/$$SHA/idls/$(IDL).json" -o idls/$(IDL).json; \
	FETCHED_AT=$$(date -u +%Y-%m-%d); \
	PROG_NAME=$$(node -p "require('./idls/$(IDL).json').metadata?.name || '$(IDL)'"); \
	PROG_VERSION=$$(node -p "require('./idls/$(IDL).json').metadata?.version || 'unknown'"); \
	printf '{\n  "url": "https://raw.githubusercontent.com/%s/%s/idls/%s.json",\n  "repository": "https://github.com/%s",\n  "commit": "%s",\n  "fetchedAt": "%s",\n  "programName": "%s",\n  "programVersion": "%s",\n  "note": "Pinned. Run `make update-idl-$(TARGET)` to refresh to the latest commit on main."\n}\n' "$(REPO)" "$$SHA" "$(IDL)" "$(REPO)" "$$SHA" "$$FETCHED_AT" "$$PROG_NAME" "$$PROG_VERSION" > idls/$(IDL).source.json
	@pnpm tsx scripts/codama.ts $(TARGET)
	@echo ""
	@echo "  IDL refreshed. Review the diff:"
	@echo "    git status"
	@echo "    git diff idls/ src/generated/"
	@echo "  Then run typecheck + the example before committing."

# ── Release ──────────────────────────────────────────────
# package.json is the source of truth; VERSION mirrors it. Local release
# targets bump, sync, commit, and tag — they never push. `push-release`
# pushes the tag, which (once the release workflow is enabled) triggers a
# clean-room build on GitHub Actions that publishes to npm with signed
# provenance. Nothing publishes from this machine.

version: ## Print current version
	@node -p "require('./package.json').version"

release-patch: _require-clean ## Bump patch version (0.1.0 -> 0.1.1)
	@$(MAKE) _bump BUMP=patch

release-minor: _require-clean ## Bump minor version (0.1.0 -> 0.2.0)
	@$(MAKE) _bump BUMP=minor

release-major: _require-clean ## Bump major version (0.1.0 -> 1.0.0)
	@$(MAKE) _bump BUMP=major

_require-clean:
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "✖ Working tree not clean. Commit or stash changes first."; \
		exit 1; \
	fi

_bump:
	@pnpm typecheck
	@pnpm build
	@npm version $(BUMP) --no-git-tag-version >/dev/null
	@node -p "require('./package.json').version" > VERSION
	@NEW_VERSION=$$(cat VERSION); \
	git add package.json VERSION; \
	git commit -m "release: v$$NEW_VERSION"; \
	git tag "v$$NEW_VERSION"; \
	echo ""; \
	echo "  Tagged v$$NEW_VERSION locally."; \
	echo "  Next:  make push-release   # push commit + tag → CI publishes to npm"

push-release: ## Push release commit + tag → CI handles the npm publish
	git push origin HEAD
	git push origin --tags
	@echo ""
	@echo "  Pushed. If the release workflow is enabled, the publish run is here:"
	@echo "    https://github.com/tylerthebuildor/surfpool-helius/actions/workflows/release.yml"

# ── Cleanup ──────────────────────────────────────────────

clean: ## Remove build artifacts
	rm -rf dist node_modules

# ── Help ─────────────────────────────────────────────────

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

.DEFAULT_GOAL := help
