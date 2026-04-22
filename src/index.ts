// Crate root. This is the public surface of surfpool-helius — everything
// a consumer can import. Files not re-exported from here are considered
// crate-internal (Rust: `pub(crate)`), even if TypeScript can't enforce it.
//
// Two consumption modes:
//
//   1. Built-in server — `createProxy(opts)` starts HTTP + WS on a port.
//      This is what the CLI and most users reach for.
//
//   2. Transport-agnostic — `createHeliusContext` + `handleJsonRpcBody`.
//      Hand the response to any third-party mock library (MSW, Nock,
//      undici MockAgent, miragejs, …) through its native handler shape.
//      See the README for integration examples. v1 is HTTP-only; WebSocket
//      subscriptions are available only through `createProxy`.

import type { HeliusContextOptions } from "./core.js";

export interface ProxyOptions extends HeliusContextOptions {
  /**
   * HTTP port the proxy listens on. WebSocket server runs on `port + 1`
   * (web3.js auto-derives WS as HTTP + 1 for localhost). Default: 8897,
   * which puts WS on 8898 — avoiding a collision with Surfpool's own
   * 8899/8900/8488.
   */
  port?: number;
}

// Built-in server.
export { createProxy } from "./server/index.js";

// Transport-agnostic primitives — for plugging surfpool-helius into any
// third-party mock library.
export {
  createHeliusContext,
  handleJsonRpcBody,
  dispatch,
  findHandler,
  handlers,
  jsonRpcError,
  jsonRpcResult,
} from "./core.js";
export type { HeliusContextOptions } from "./core.js";
export { createFixtureUpstream } from "./fixtures.js";
export type { FixtureUpstreamOptions } from "./fixtures.js";

// Context + JSON-RPC types.
export type {
  RequestContext,
  ResolvedContextOptions,
  Handler,
  JsonRpcResponse,
  JsonRpcSuccess,
  JsonRpcFailure,
} from "./context.js";

// Upstream + cache — injectable pieces of the context.
export type { UpstreamClient, AccountData } from "./upstream.js";
export { createUpstreamClient } from "./upstream.js";
export type { CacheStore, EditionRecord, SearchAssetsFilter, SortBy, SortDirection, TokenType } from "./cache/index.js";
export { createMemoryCache } from "./cache/index.js";

// Decoders.
export type { AccountDecoder, DasAsset } from "./decoders/index.js";
export { mplCoreDecoder, tokenMetadataDecoder } from "./decoders/index.js";

// Compat manifest (introspection).
export type {
  CompatLevel,
  Namespace,
  MethodEntry,
  ManifestSummary,
} from "./compat/index.js";
export { manifest, summarize } from "./compat/index.js";
