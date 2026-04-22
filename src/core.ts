// Transport-agnostic surfpool-helius core. Everything here is independent
// of the built-in HTTP server — it's what third-party mock libraries (MSW,
// Nock, undici MockAgent, miragejs, …) plug into. The only thing a consumer
// has to do is hand a request body to `handleJsonRpcBody` and return the
// result; if they get back null, the method isn't ours and they should use
// their library's passthrough.

import type { Address } from "@solana/kit";
import type { AccountDecoder } from "./decoders/index.js";
import type { CacheStore } from "./cache/index.js";
import type { CnftStore } from "./cnft/index.js";
import type { UpstreamClient } from "./upstream.js";
import type { JsonRpcResponse, RequestContext, ResolvedContextOptions } from "./context.js";
import { mplCoreDecoder, tokenMetadataDecoder } from "./decoders/index.js";
import { createMemoryCache } from "./cache/index.js";
import { createCnftMemoryStore, indexTree } from "./cnft/index.js";
import { createUpstreamClient } from "./upstream.js";
import { dispatch } from "./router.js";

export interface HeliusContextOptions {
  /** Upstream Solana RPC URL. Only used when no custom `upstream` is provided. Default: http://127.0.0.1:8899. */
  upstreamUrl?: string;
  /**
   * Upstream Solana WebSocket URL. Read by the WS polyfill for forwarding
   * subscriptions we don't handle locally. Default: derived from
   * `upstreamUrl`'s host on port 8900 (Surfpool's default WS port). Set
   * this explicitly when the upstream WS is on a different host or when
   * you need `wss://`.
   */
  upstreamWsUrl?: string;
  /** Timeout for the default upstream HTTP client. Ignored when `upstream` is injected. Default: 10000. */
  rpcTimeoutMs?: number;
  /** Account decoders. Default: [mplCoreDecoder, tokenMetadataDecoder]. Pass [] to disable DAS. */
  decoders?: AccountDecoder[];
  /** Inject a custom UpstreamClient — e.g. `createFixtureUpstream()` for tests, or a recorded fixture. Skips the default HTTP client. */
  upstream?: UpstreamClient;
  /** Inject a custom CacheStore. Default: createMemoryCache(). */
  cache?: CacheStore;
  /** Inject a custom CnftStore. Default: createCnftMemoryStore(). */
  cnft?: CnftStore;
  /**
   * Bubblegum tree pubkeys to backfill and keep fresh. cNFTs on any
   * listed tree resolve via `getAsset` + `getAssetProof` without an
   * upstream hop. Trees are indexed once at startup; call
   * `surfpoolHeliusIndexTree` (runtime method) to refresh or add trees
   * while the proxy is running. Default: [] (cNFT support disabled
   * until a tree is indexed).
   */
  indexTrees?: string[];
}

// Default the WS URL to the same host as the HTTP upstream on port 8900.
// If the upstream URL is unparseable, fall back to 127.0.0.1 so local dev
// stays working even if someone passes a garbage string.
function deriveDefaultWsUrl(upstreamUrl: string): string {
  try {
    const host = new URL(upstreamUrl).hostname;
    return `ws://${host}:8900`;
  } catch {
    return "ws://127.0.0.1:8900";
  }
}

export function resolveContextOptions(
  opts: HeliusContextOptions,
): ResolvedContextOptions {
  const upstreamUrl = opts.upstreamUrl ?? "http://127.0.0.1:8899";
  return {
    upstreamUrl,
    upstreamWsUrl: opts.upstreamWsUrl ?? deriveDefaultWsUrl(upstreamUrl),
    rpcTimeoutMs: opts.rpcTimeoutMs ?? 10_000,
    decoders: opts.decoders ?? [mplCoreDecoder, tokenMetadataDecoder],
  };
}

// Build a RequestContext that can be shared across any transport. Call this
// once per mock instance; do not share a single context across independently
// running mocks — `cache` and `cnft` are stateful and pollution between tests
// will bite.
//
// If `options.indexTrees` is non-empty, an initial backfill for each tree is
// kicked off **in the background** — the returned context is usable
// immediately; cNFT lookups just return null until the backfill catches up.
// Failures are logged, never thrown; a bad tree pubkey doesn't break the
// rest of the proxy.
export function createHeliusContext(
  options: HeliusContextOptions = {},
): RequestContext {
  const opts = resolveContextOptions(options);
  const upstream =
    options.upstream ?? createUpstreamClient(opts.upstreamUrl, opts.rpcTimeoutMs);
  const cache = options.cache ?? createMemoryCache();
  const cnft = options.cnft ?? createCnftMemoryStore();
  const ctx: RequestContext = { opts, upstream, cache, cnft, decoders: opts.decoders };

  if (options.indexTrees && options.indexTrees.length > 0) {
    void backfillTreesInBackground(ctx, options.indexTrees);
  }

  return ctx;
}

async function backfillTreesInBackground(
  ctx: RequestContext,
  trees: string[],
): Promise<void> {
  for (const tree of trees) {
    try {
      const result = await indexTree(
        { upstream: ctx.upstream, store: ctx.cnft },
        tree as Address,
      );
      console.log(
        `[surfpool-helius cnft] Indexed tree ${tree}: ${result.applied} ix(s) applied, cursor=${result.newestApplied ?? "(none)"}`,
      );
    } catch (err) {
      console.error(
        `[surfpool-helius cnft] Failed to index tree ${tree}:`,
        err instanceof Error ? err.message : err,
      );
    }
  }
}

// Parse a raw JSON-RPC request body and dispatch it. Returns the response
// object for methods surfpool-helius implements, or null when the method
// falls through (unknown method, non-JSON body, missing method field) — in
// which case the caller should defer to its transport's passthrough.
//
// Accepts a string, a Buffer/Uint8Array, or an already-parsed object. The
// last form is for libraries (MSW, undici) that hand you a parsed request.
export async function handleJsonRpcBody(
  ctx: RequestContext,
  body: string | Uint8Array | { method?: unknown; id?: unknown; params?: unknown },
): Promise<JsonRpcResponse | null> {
  let parsed: { method?: unknown; id?: unknown; params?: unknown };

  if (typeof body === "string") {
    try {
      parsed = JSON.parse(body);
    } catch {
      return null;
    }
  } else if (body instanceof Uint8Array) {
    try {
      parsed = JSON.parse(Buffer.from(body).toString("utf8"));
    } catch {
      return null;
    }
  } else if (body && typeof body === "object") {
    parsed = body;
  } else {
    return null;
  }

  const { method, id, params } = parsed;
  if (typeof method !== "string") return null;

  return dispatch(ctx, method, params, id);
}

// Re-export for convenience so consumers writing custom adapters can reach
// the lower-level primitives without chasing subpaths.
export { dispatch, findHandler } from "./router.js";
export { handlers } from "./namespaces/index.js";

// Convenience re-export so `handleJsonRpcBody` + error helper live together.
export { jsonRpcError, jsonRpcResult } from "./context.js";
