// The RequestContext is the shared state passed to every namespace handler.
// Handlers read from `cache`, call upstream Solana RPC via `upstream`, and
// inspect `decoders` + `opts` when they need configuration. Keeping this as
// one narrow type means each handler file stays small and dependency-free.

import type { AccountDecoder } from "./decoders/index.js";
import type { CacheStore } from "./cache/index.js";
import type { UpstreamClient } from "./upstream.js";

export interface ResolvedOptions {
  port: number;
  upstreamUrl: string;
  upstreamWsPort: number;
  rpcTimeoutMs: number;
  decoders: AccountDecoder[];
}

export interface RequestContext {
  opts: ResolvedOptions;
  upstream: UpstreamClient;
  cache: CacheStore;
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
// looks up the method name, calls the handler, and writes the returned
// JsonRpcResponse straight to the HTTP response body.
export type Handler = (
  ctx: RequestContext,
  params: unknown,
  id: unknown,
) => Promise<JsonRpcResponse>;

export function resolveOptions(opts: {
  port?: number;
  upstreamUrl?: string;
  upstreamWsPort?: number;
  rpcTimeoutMs?: number;
  decoders?: AccountDecoder[];
}, defaultDecoders: AccountDecoder[]): ResolvedOptions {
  return {
    port: opts.port ?? 8897,
    upstreamUrl: opts.upstreamUrl ?? "http://127.0.0.1:8899",
    upstreamWsPort: opts.upstreamWsPort ?? 8900,
    rpcTimeoutMs: opts.rpcTimeoutMs ?? 10_000,
    decoders: opts.decoders ?? defaultDecoders,
  };
}

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
