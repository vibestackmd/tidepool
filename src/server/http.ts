// HTTP server. Handles JSON-RPC over POST, CORS, and the passthrough
// path that forwards anything the router doesn't recognize to Surfpool
// unchanged. Keeping the server thin means every method's behavior lives
// in its namespace handler, not here.

import http from "node:http";
import type { RequestContext } from "../context.js";
import { dispatch } from "../router.js";
import { logUpstreamError } from "./logging.js";

export function createHttpServer(ctx: RequestContext): http.Server {
  return http.createServer(async (req, res) => {
    res.setHeader("Access-Control-Allow-Origin", "*");
    res.setHeader("Access-Control-Allow-Methods", "POST, OPTIONS");
    res.setHeader("Access-Control-Allow-Headers", "*");
    if (req.method === "OPTIONS") {
      res.writeHead(204);
      res.end();
      return;
    }

    const chunks: Buffer[] = [];
    for await (const chunk of req) chunks.push(chunk as Buffer);
    const body = Buffer.concat(chunks).toString();

    let parsed: { method?: string; id?: unknown; params?: unknown };
    try {
      parsed = JSON.parse(body);
    } catch {
      // Malformed JSON — let Surfpool reject it and mirror its error.
      proxyToUpstream(ctx, res, body);
      return;
    }

    const { method, id, params } = parsed;

    if (method) {
      const response = await dispatch(ctx, method, params, id);
      if (response) {
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify(response));
        return;
      }
    }

    proxyToUpstream(ctx, res, body);
  });
}

function proxyToUpstream(
  ctx: RequestContext,
  res: http.ServerResponse,
  body: string,
): void {
  const proxyReq = http.request(
    ctx.opts.upstreamUrl,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      timeout: ctx.opts.rpcTimeoutMs,
    },
    (proxyRes) => {
      const headers = { ...proxyRes.headers };
      headers["access-control-allow-origin"] = "*";
      headers["access-control-allow-methods"] = "POST, OPTIONS";
      headers["access-control-allow-headers"] = "*";
      res.writeHead(proxyRes.statusCode ?? 200, headers);
      proxyRes.pipe(res);
    },
  );
  proxyReq.on("timeout", () => {
    proxyReq.destroy();
    logUpstreamError(`Request timed out after ${ctx.opts.rpcTimeoutMs}ms`);
    if (!res.headersSent) {
      res.writeHead(504, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          jsonrpc: "2.0",
          id: null,
          error: { code: -32000, message: "Surfpool request timed out" },
        }),
      );
    }
  });
  proxyReq.on("error", (err) => {
    logUpstreamError(err.message);
    if (!res.headersSent) {
      res.writeHead(502, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          jsonrpc: "2.0",
          id: null,
          error: {
            code: -32000,
            message: `Surfpool unreachable: ${err.message}`,
          },
        }),
      );
    }
  });
  proxyReq.write(body);
  proxyReq.end();
}
