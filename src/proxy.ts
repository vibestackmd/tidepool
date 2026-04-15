/**
 * surfpool-helius — a Helius-compatible RPC proxy on top of Surfpool.
 *
 * Sits between your app and Surfpool. Intercepts Helius DAS methods
 * (`getAsset`, `searchAssets`) and translates them into real on-chain reads.
 * Proxies every other RPC method straight through to Surfpool.
 *
 * Also runs a WebSocket server that polyfills `signatureSubscribe` via HTTP
 * polling — Surfpool's WS doesn't support subscription methods, but
 * `connection.confirmTransaction()` and most wallet flows depend on it.
 *
 * Point your app's RPC URL at this proxy and the full Helius dev loop works
 * locally with zero client changes.
 */

import http from "node:http";
import { WebSocket, WebSocketServer } from "ws";
import type { AccountDecoder, DasAsset } from "./decoders/index.js";
import { mplCoreDecoder } from "./decoders/index.js";

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
  /** Account decoders. Default: [mplCoreDecoder]. Pass [] to disable DAS entirely. */
  decoders?: AccountDecoder[];
}

interface ResolvedOptions {
  port: number;
  upstreamUrl: string;
  upstreamWsPort: number;
  rpcTimeoutMs: number;
  decoders: AccountDecoder[];
}

function resolveOptions(opts: ProxyOptions): ResolvedOptions {
  return {
    port: opts.port ?? 8897,
    upstreamUrl: opts.upstreamUrl ?? "http://127.0.0.1:8899",
    upstreamWsPort: opts.upstreamWsPort ?? 8900,
    rpcTimeoutMs: opts.rpcTimeoutMs ?? 10_000,
    decoders: opts.decoders ?? [mplCoreDecoder],
  };
}

// ─── In-memory asset cache ───────────────────────────────────────────────────

// `searchAssets` reads from this cache. It's populated as a side effect of
// `getAsset` calls — so anything you've ever fetched is searchable. This
// sidesteps the `getProgramAccounts` trap against Surfpool's mainnet-forked
// upstream, where scanning millions of accounts would be hopeless.
const assetCache = new Map<string, DasAsset>();

// ─── Error logging ──────────────────────────────────────────────────────────

let lastUpstreamError = 0;
function logUpstreamError(detail: string) {
  const now = Date.now();
  if (now - lastUpstreamError < 10_000) return;
  lastUpstreamError = now;
  const RED = "\x1b[31m";
  const YELLOW = "\x1b[33m";
  const DIM = "\x1b[2m";
  const BOLD = "\x1b[1m";
  const R = "\x1b[0m";
  console.error(`
${RED}${BOLD}  ════════════════════════════════════════════════════${R}
${RED}${BOLD}  SURFPOOL NOT RESPONDING${R}
${RED}  ${detail}${R}

${YELLOW}  Surfpool may have crashed or stalled.${R}
${YELLOW}  Is it running?  docker compose up -d${R}
${DIM}  ════════════════════════════════════════════════════${R}
`);
}

// ─── Upstream RPC helpers ───────────────────────────────────────────────────

async function rpcCall(
  upstreamUrl: string,
  timeoutMs: number,
  method: string,
  params: unknown[]
): Promise<unknown> {
  const body = JSON.stringify({ jsonrpc: "2.0", id: 1, method, params });
  let resp: Response;
  try {
    resp = await fetch(upstreamUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body,
      signal: AbortSignal.timeout(timeoutMs),
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : "Unknown error";
    logUpstreamError(msg);
    throw new Error(`Surfpool unreachable: ${msg}`);
  }
  const json = (await resp.json()) as { result?: unknown; error?: unknown };
  if (json.error) throw new Error(JSON.stringify(json.error));
  return json.result;
}

interface AccountInfoResult {
  value: {
    data: [string, string];
    owner: string;
    lamports: number;
    executable: boolean;
    rentEpoch: number;
  } | null;
}

async function getAccount(
  upstreamUrl: string,
  timeoutMs: number,
  address: string
): Promise<{ data: Uint8Array; owner: string; lamports: number } | null> {
  const result = (await rpcCall(upstreamUrl, timeoutMs, "getAccountInfo", [
    address,
    { encoding: "base64" },
  ])) as AccountInfoResult;
  if (!result?.value) return null;

  const data = Buffer.from(result.value.data[0], "base64");
  return {
    data: new Uint8Array(data),
    owner: result.value.owner,
    lamports: result.value.lamports,
  };
}

// ─── Asset fetch + cache ────────────────────────────────────────────────────

async function fetchAndCache(
  opts: ResolvedOptions,
  address: string
): Promise<DasAsset | null> {
  const account = await getAccount(opts.upstreamUrl, opts.rpcTimeoutMs, address);
  if (!account) return null;

  const decoder = opts.decoders.find((d) => d.programId === account.owner);
  if (!decoder) return null;

  const asset = await decoder.decode(address, account.data);
  if (asset) {
    assetCache.set(address, asset);
  }
  return asset;
}

// ─── DAS method handlers ────────────────────────────────────────────────────

async function handleGetAsset(
  opts: ResolvedOptions,
  params: { id: string },
  id: unknown
): Promise<object> {
  const asset = await fetchAndCache(opts, params.id);
  if (!asset) {
    return {
      jsonrpc: "2.0",
      id,
      error: { code: -32000, message: "Asset not found" },
    };
  }
  return { jsonrpc: "2.0", id, result: asset };
}

interface SearchAssetsParams {
  ownerAddress?: string;
  authorityAddress?: string;
  interface?: string;
  grouping?: [string, string];
  page?: number;
  limit?: number;
}

function handleSearchAssets(params: SearchAssetsParams, id: unknown): object {
  let items = Array.from(assetCache.values());

  const ownerFilter = params.ownerAddress ?? params.authorityAddress;
  if (ownerFilter) {
    items = items.filter((a) => a.ownership.owner === ownerFilter);
  }

  if (params.interface) {
    items = items.filter((a) => a.interface === params.interface);
  }

  if (params.grouping) {
    const [, collectionId] = params.grouping;
    items = items.filter((a) =>
      a.grouping.some((g) => g.group_value === collectionId)
    );
  }

  const page = params.page ?? 1;
  const limit = params.limit ?? 20;
  const start = (page - 1) * limit;
  const paged = items.slice(start, start + limit);

  return {
    jsonrpc: "2.0",
    id,
    result: { total: items.length, limit, page, items: paged },
  };
}

// ─── WebSocket server with signatureSubscribe polyfill ──────────────────────

let nextSubId = 1;

interface SigSubscription {
  subId: number;
  signature: string;
  commitment: string;
  timer: ReturnType<typeof setInterval>;
}

function setupWsServer(opts: ResolvedOptions, listenPort: number): WebSocketServer {
  const wss = new WebSocketServer({ port: listenPort });

  wss.on("connection", (clientWs) => {
    const subscriptions = new Map<number, SigSubscription>();

    // Lazy upstream WS connection — only created when we need to forward a
    // method we don't polyfill. Avoids noise when Surfpool WS is down (which is
    // fine, since the polyfilled methods handle the critical paths).
    let upstreamWs: WebSocket | null = null;

    function getOrCreateUpstreamWs(): WebSocket | null {
      if (upstreamWs && upstreamWs.readyState === WebSocket.OPEN) return upstreamWs;
      if (upstreamWs && upstreamWs.readyState === WebSocket.CONNECTING) return upstreamWs;

      try {
        upstreamWs = new WebSocket(`ws://127.0.0.1:${opts.upstreamWsPort}`);
        upstreamWs.on("message", (data) => {
          if (clientWs.readyState === WebSocket.OPEN) {
            clientWs.send(data.toString());
          }
        });
        upstreamWs.on("error", () => {
          // Surfpool WS may not support the forwarded method — swallow.
        });
        upstreamWs.on("close", () => {
          upstreamWs = null;
        });
        return upstreamWs;
      } catch {
        return null;
      }
    }

    function cleanupAll() {
      for (const sub of subscriptions.values()) {
        clearInterval(sub.timer);
      }
      subscriptions.clear();
      if (
        upstreamWs &&
        (upstreamWs.readyState === WebSocket.OPEN ||
          upstreamWs.readyState === WebSocket.CONNECTING)
      ) {
        upstreamWs.close();
      }
      upstreamWs = null;
    }

    clientWs.on("message", (raw) => {
      let msg: { jsonrpc: string; id: number; method: string; params: unknown[] };
      try {
        msg = JSON.parse(raw.toString());
      } catch {
        return; // malformed JSON — drop
      }

      // ── signatureSubscribe: polyfill via HTTP polling ─────────────
      if (msg.method === "signatureSubscribe") {
        const signature = msg.params[0] as string;
        const subOpts = (msg.params[1] ?? {}) as { commitment?: string };
        const commitment = subOpts.commitment ?? "finalized";
        const subId = nextSubId++;

        clientWs.send(
          JSON.stringify({ jsonrpc: "2.0", result: subId, id: msg.id })
        );

        const timer = setInterval(async () => {
          if (clientWs.readyState !== WebSocket.OPEN) {
            clearInterval(timer);
            subscriptions.delete(subId);
            return;
          }

          try {
            const resp = await fetch(opts.upstreamUrl, {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({
                jsonrpc: "2.0",
                id: 1,
                method: "getSignatureStatuses",
                params: [[signature], { searchTransactionHistory: true }],
              }),
              signal: AbortSignal.timeout(opts.rpcTimeoutMs),
            });
            const json = (await resp.json()) as {
              result?: {
                value: Array<{ err: unknown; confirmationStatus: string } | null>;
              };
            };
            const status = json.result?.value?.[0];
            if (!status) return;

            const levels = ["processed", "confirmed", "finalized"];
            const target = levels.indexOf(commitment);
            const current = levels.indexOf(status.confirmationStatus);
            if (current < target) return;

            if (clientWs.readyState === WebSocket.OPEN) {
              clientWs.send(
                JSON.stringify({
                  jsonrpc: "2.0",
                  method: "signatureNotification",
                  params: {
                    result: {
                      context: { slot: 0 },
                      value: { err: status.err ?? null },
                    },
                    subscription: subId,
                  },
                })
              );
            }

            clearInterval(timer);
            subscriptions.delete(subId);
          } catch {
            // Upstream temporarily unreachable — keep retrying.
          }
        }, 400);

        subscriptions.set(subId, { subId, signature, commitment, timer });
        return;
      }

      // ── signatureUnsubscribe: cancel polling ──────────────────────
      if (msg.method === "signatureUnsubscribe") {
        const subId = msg.params[0] as number;
        const sub = subscriptions.get(subId);
        if (sub) {
          clearInterval(sub.timer);
          subscriptions.delete(subId);
        }
        clientWs.send(
          JSON.stringify({ jsonrpc: "2.0", result: !!sub, id: msg.id })
        );
        return;
      }

      // ── Everything else: forward to Surfpool WS ───────────────────
      const upstream = getOrCreateUpstreamWs();
      if (upstream && upstream.readyState === WebSocket.OPEN) {
        upstream.send(raw.toString());
      }
    });

    clientWs.on("close", cleanupAll);
    clientWs.on("error", cleanupAll);
  });

  return wss;
}

// ─── HTTP server ────────────────────────────────────────────────────────────

function proxyToUpstream(
  opts: ResolvedOptions,
  res: http.ServerResponse,
  body: string
): void {
  const proxyReq = http.request(
    opts.upstreamUrl,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      timeout: opts.rpcTimeoutMs,
    },
    (proxyRes) => {
      const headers = { ...proxyRes.headers };
      headers["access-control-allow-origin"] = "*";
      headers["access-control-allow-methods"] = "POST, OPTIONS";
      headers["access-control-allow-headers"] = "*";
      res.writeHead(proxyRes.statusCode ?? 200, headers);
      proxyRes.pipe(res);
    }
  );
  proxyReq.on("timeout", () => {
    proxyReq.destroy();
    logUpstreamError(`Request timed out after ${opts.rpcTimeoutMs}ms`);
    if (!res.headersSent) {
      res.writeHead(504, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          jsonrpc: "2.0",
          id: null,
          error: { code: -32000, message: "Surfpool request timed out" },
        })
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
          error: { code: -32000, message: `Surfpool unreachable: ${err.message}` },
        })
      );
    }
  });
  proxyReq.write(body);
  proxyReq.end();
}

export function createProxy(options: ProxyOptions = {}): Promise<http.Server> {
  const opts = resolveOptions(options);

  return new Promise((resolve, reject) => {
    const server = http.createServer(async (req, res) => {
      // CORS — allow browser requests from any origin.
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
        proxyToUpstream(opts, res, body);
        return;
      }

      const { method, id, params } = parsed;

      try {
        if (method === "getAsset") {
          const result = await handleGetAsset(opts, params as { id: string }, id);
          res.writeHead(200, { "Content-Type": "application/json" });
          res.end(JSON.stringify(result));
          return;
        }

        if (method === "searchAssets") {
          const result = handleSearchAssets(params as SearchAssetsParams, id);
          res.writeHead(200, { "Content-Type": "application/json" });
          res.end(JSON.stringify(result));
          return;
        }
      } catch (err) {
        console.error(`[surfpool-helius] Error handling ${method}:`, err);
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({
            jsonrpc: "2.0",
            id,
            error: { code: -32000, message: String(err) },
          })
        );
        return;
      }

      proxyToUpstream(opts, res, body);
    });

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
      const wss = setupWsServer(opts, wsPort);

      wss.on("error", (err: NodeJS.ErrnoException) => {
        console.error(`[surfpool-helius] WS server error on port ${wsPort}: ${err.message}`);
      });

      server.on("close", () => {
        for (const client of wss.clients) client.close();
        wss.close();
      });

      const decoderNames = opts.decoders.map((d) => d.name).join(", ") || "(none)";
      console.log(`[surfpool-helius] HTTP on :${opts.port}  WS on :${wsPort}`);
      console.log(`[surfpool-helius] Upstream: ${opts.upstreamUrl}  (WS :${opts.upstreamWsPort})`);
      console.log(`[surfpool-helius] Decoders: ${decoderNames}`);
      resolve(server);
    });
  });
}
