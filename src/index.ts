// Crate root. This is the public surface of the surfpool-helius library
// — everything a consumer can import. Files not re-exported from here
// are considered crate-internal (Rust: `pub(crate)`), even if TypeScript
// can't enforce that.
//
// Public surface:
//   - createProxy              — the main entry point
//   - ProxyOptions             — the options shape
//   - AccountDecoder, DasAsset — for users writing custom decoders
//   - mplCoreDecoder,          — the default decoders, re-exported so
//     tokenMetadataDecoder        advanced users can compose them
//   - Manifest types           — for consumers that want to introspect
//                                 the compat surface programmatically

import type { AccountDecoder } from "./decoders/index.js";

export interface ProxyOptions {
  /**
   * HTTP port the proxy listens on. WebSocket server runs on `port + 1`
   * (web3.js auto-derives WS as HTTP + 1 for localhost). Default: 8897,
   * which puts WS on 8898 — avoiding a collision with Surfpool's own
   * 8899/8900/8488.
   */
  port?: number;
  /** Upstream Surfpool RPC URL. Default: http://127.0.0.1:8899. */
  upstreamUrl?: string;
  /** Upstream Surfpool WebSocket port. Default: 8900. */
  upstreamWsPort?: number;
  /** Timeout for upstream RPC calls in milliseconds. Default: 10000. */
  rpcTimeoutMs?: number;
  /** Account decoders. Default: [mplCoreDecoder, tokenMetadataDecoder]. Pass [] to disable DAS entirely. */
  decoders?: AccountDecoder[];
}

export { createProxy } from "./server/index.js";
export type { AccountDecoder, DasAsset } from "./decoders/index.js";
export { mplCoreDecoder, tokenMetadataDecoder } from "./decoders/index.js";
export type {
  CompatLevel,
  Namespace,
  MethodEntry,
  ManifestSummary,
} from "./compat/index.js";
export { manifest, summarize } from "./compat/index.js";
