// Server module surface. Exposes createProxy — the entry point that wires
// the built-in HTTP + WebSocket server to a shared RequestContext. Third-
// party mock libraries bypass this file entirely and hook the context
// primitives in `src/core.ts` into their own transport.

import http from "node:http";
import type { ProxyOptions } from "../index.js";
import { createHeliusContext } from "../core.js";
import { createHttpServer } from "./http.js";
import { createWsServer } from "./ws.js";

export function createProxy(options: ProxyOptions = {}): Promise<http.Server> {
  const port = options.port ?? 8897;
  const ctx = createHeliusContext(options);

  return new Promise((resolve, reject) => {
    const server = createHttpServer(ctx);

    server.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "EADDRINUSE") {
        console.error(`[surfpool-helius] Port ${port} already in use`);
      } else {
        console.error(`[surfpool-helius] Server error: ${err.message}`);
      }
      reject(err);
    });

    server.listen(port, () => {
      const wsPort = port + 1;
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

      const decoderNames = ctx.decoders.map((d) => d.name).join(", ") || "(none)";
      console.log(`[surfpool-helius] HTTP on :${port}  WS on :${wsPort}`);
      console.log(
        `[surfpool-helius] Upstream: ${ctx.opts.upstreamUrl}  (WS: ${ctx.opts.upstreamWsUrl})`,
      );
      console.log(`[surfpool-helius] Decoders: ${decoderNames}`);
      resolve(server);
    });
  });
}
