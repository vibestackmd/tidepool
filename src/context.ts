// The RequestContext is the shared state passed to every namespace handler.
// Handlers read from `cache`, call upstream Solana RPC via `upstream`, and
// inspect `decoders` + `opts` when they need configuration. Keeping this as
// one narrow type means each handler file stays small and dependency-free.

import type { AccountDecoder } from "./decoders/index.js";
import type { CacheStore } from "./cache/index.js";
import type { CnftStore } from "./cnft/index.js";
import type { UpstreamClient } from "./upstream.js";

// Options that shape the request context itself. These are transport-agnostic
// — they apply equally whether the context is hosted by the built-in server
// or by a third-party mock library (MSW, Nock, undici MockAgent, etc).
export interface ResolvedContextOptions {
  upstreamUrl: string;
  upstreamWsUrl: string;
  rpcTimeoutMs: number;
  decoders: AccountDecoder[];
}

export interface RequestContext {
  opts: ResolvedContextOptions;
  upstream: UpstreamClient;
  cache: CacheStore;
  cnft: CnftStore;
  decoders: AccountDecoder[];
}

export type JsonRpcSuccess = {
  jsonrpc: "2.0";
  id: unknown;
  result: unknown;
};

export type JsonRpcFailure = {
  jsonrpc: "2.0";
  id: unknown;
  error: { code: number; message: string; data?: unknown };
};

export type JsonRpcResponse = JsonRpcSuccess | JsonRpcFailure;

// Every handler in every namespace has this exact signature. The router
// looks up the method name, calls the handler, and the caller writes the
// returned JsonRpcResponse to whatever transport it owns (HTTP server,
// MSW handler, Nock reply, etc).
export type Handler = (
  ctx: RequestContext,
  params: unknown,
  id: unknown,
) => Promise<JsonRpcResponse>;

export function jsonRpcError(
  id: unknown,
  code: number,
  message: string,
  data?: unknown,
): JsonRpcFailure {
  return {
    jsonrpc: "2.0",
    id,
    error: data === undefined ? { code, message } : { code, message, data },
  };
}

export function jsonRpcResult(id: unknown, result: unknown): JsonRpcSuccess {
  return { jsonrpc: "2.0", id, result };
}
