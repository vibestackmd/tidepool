// WebSocket server. Owns per-connection state (subscriptions, lazy
// upstream WS) and dispatches signatureSubscribe/signatureUnsubscribe to
// the WS namespace polyfill. Everything else forwards to Surfpool's WS
// when available — Surfpool doesn't implement subscriptions, so most
// forwarded messages are silently dropped on that side, which is fine.

import { WebSocket, WebSocketServer } from "ws";
import type { RequestContext } from "../context.js";
import {
  startSignatureSubscribe,
  stopSignatureSubscribe,
  type SigSubscription,
} from "../namespaces/ws/index.js";

let nextSubId = 1;

export function createWsServer(
  ctx: RequestContext,
  listenPort: number,
): WebSocketServer {
  const wss = new WebSocketServer({ port: listenPort });

  wss.on("connection", (clientWs) => {
    const subscriptions = new Map<number, SigSubscription>();

    // Lazy upstream WS — only created when we need to forward a method
    // we don't polyfill. Avoids noise when Surfpool WS is down, which is
    // fine because the polyfilled methods handle the critical paths.
    let upstreamWs: WebSocket | null = null;

    function getOrCreateUpstreamWs(): WebSocket | null {
      if (upstreamWs && upstreamWs.readyState === WebSocket.OPEN) return upstreamWs;
      if (upstreamWs && upstreamWs.readyState === WebSocket.CONNECTING) return upstreamWs;

      try {
        upstreamWs = new WebSocket(`ws://127.0.0.1:${ctx.opts.upstreamWsPort}`);
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
        stopSignatureSubscribe(sub);
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

      if (msg.method === "signatureSubscribe") {
        const subId = nextSubId++;
        const sub = startSignatureSubscribe({
          clientWs,
          upstreamUrl: ctx.opts.upstreamUrl,
          rpcTimeoutMs: ctx.opts.rpcTimeoutMs,
          msgId: msg.id,
          params: msg.params,
          subId,
        });
        subscriptions.set(subId, sub);
        return;
      }

      if (msg.method === "signatureUnsubscribe") {
        const subId = msg.params[0] as number;
        const sub = subscriptions.get(subId);
        if (sub) {
          stopSignatureSubscribe(sub);
          subscriptions.delete(subId);
        }
        clientWs.send(
          JSON.stringify({ jsonrpc: "2.0", result: !!sub, id: msg.id }),
        );
        return;
      }

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
