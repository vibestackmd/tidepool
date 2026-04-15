// Server module surface. Exposes createProxy — the single public entry
// point that wires the HTTP server, WebSocket server, cache, upstream
// client, and request context together, then starts listening.

import http from "node:http";
import type { ProxyOptions } from "../index.js";
import type { RequestContext } from "../context.js";
import { resolveOptions } from "../context.js";
import { createUpstreamClient } from "../upstream.js";
import { createMemoryCache } from "../cache/index.js";
import { mplCoreDecoder } from "../decoders/index.js";
import { createHttpServer } from "./http.js";
import { createWsServer } from "./ws.js";

export function createProxy(options: ProxyOptions = {}): Promise<http.Server> {
  const opts = resolveOptions(options, [mplCoreDecoder]);

  const ctx: RequestContext = {
    opts,
    upstream: createUpstreamClient(opts.upstreamUrl, opts.rpcTimeoutMs),
    cache: createMemoryCache(),
    decoders: opts.decoders,
  };

  return new Promise((resolve, reject) => {
    const server = createHttpServer(ctx);

    server.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "EADDRINUSE") {
        console.error(`[surfpool-helius] Port ${opts.port} already in use`);
      } else {
        console.error(`[surfpool-helius] Server error: ${err.message}`);
      }
      reject(err);
    });

    server.listen(opts.port, () => {
      const wsPort = opts.port + 1;
      const wss = createWsServer(ctx, wsPort);

      wss.on("error", (err: NodeJS.ErrnoException) => {
        console.error(
          `[surfpool-helius] WS server error on port ${wsPort}: ${err.message}`,
        );
      });

      server.on("close", () => {
        for (const client of wss.clients) client.close();
        wss.close();
      });

      const decoderNames = opts.decoders.map((d) => d.name).join(", ") || "(none)";
      console.log(
        `[surfpool-helius] HTTP on :${opts.port}  WS on :${wsPort}`,
      );
      console.log(
        `[surfpool-helius] Upstream: ${opts.upstreamUrl}  (WS :${opts.upstreamWsPort})`,
      );
      console.log(`[surfpool-helius] Decoders: ${decoderNames}`);
      resolve(server);
    });
  });
}
