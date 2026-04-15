// signatureSubscribe polyfill. Surfpool's WebSocket server doesn't support
// subscription methods, so `confirmTransaction()` in web3.js and UMI's
// `sendAndConfirm()` both hang without this. We fake the subscription by
// polling `getSignatureStatuses` over HTTP and emitting a signatureNotification
// when the target commitment level is reached.
//
// The polling interval (400ms) is tuned to be roughly one slot — fast
// enough to feel native, slow enough not to hammer Surfpool.

import { WebSocket } from "ws";

export interface SigSubscription {
  subId: number;
  timer: ReturnType<typeof setInterval>;
}

export interface SubscribeInput {
  clientWs: WebSocket;
  upstreamUrl: string;
  rpcTimeoutMs: number;
  msgId: unknown;
  params: unknown[];
  subId: number;
}

const COMMITMENT_LEVELS = ["processed", "confirmed", "finalized"] as const;

export function startSignatureSubscribe(input: SubscribeInput): SigSubscription {
  const { clientWs, upstreamUrl, rpcTimeoutMs, msgId, params, subId } = input;

  const signature = params[0] as string;
  const subOpts = (params[1] ?? {}) as { commitment?: string };
  const commitment = subOpts.commitment ?? "finalized";

  // Ack the subscription synchronously — the client expects this before
  // any notification arrives.
  clientWs.send(
    JSON.stringify({ jsonrpc: "2.0", result: subId, id: msgId }),
  );

  const timer = setInterval(async () => {
    if (clientWs.readyState !== WebSocket.OPEN) {
      clearInterval(timer);
      return;
    }

    try {
      const resp = await fetch(upstreamUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: 1,
          method: "getSignatureStatuses",
          params: [[signature], { searchTransactionHistory: true }],
        }),
        signal: AbortSignal.timeout(rpcTimeoutMs),
      });
      const json = (await resp.json()) as {
        result?: {
          value: Array<{ err: unknown; confirmationStatus: string } | null>;
        };
      };
      const status = json.result?.value?.[0];
      if (!status) return;

      const target = COMMITMENT_LEVELS.indexOf(
        commitment as typeof COMMITMENT_LEVELS[number],
      );
      const current = COMMITMENT_LEVELS.indexOf(
        status.confirmationStatus as typeof COMMITMENT_LEVELS[number],
      );
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
          }),
        );
      }

      clearInterval(timer);
    } catch {
      // Upstream temporarily unreachable — keep retrying.
    }
  }, 400);

  return { subId, timer };
}

export function stopSignatureSubscribe(sub: SigSubscription): void {
  clearInterval(sub.timer);
}
