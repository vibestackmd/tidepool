// Upstream Solana RPC client. Wraps `fetch` against the Surfpool URL so
// handlers don't have to care about HTTP, timeouts, or error translation.
// All calls flow through here — that's what makes the proxy's upstream
// error logging meaningful (one place to observe failures).

import { logUpstreamError } from "./server/logging.js";

export interface AccountData {
  data: Uint8Array;
  owner: string;
  lamports: number;
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

export interface UpstreamClient {
  rpcCall(method: string, params: unknown[]): Promise<unknown>;
  getAccount(address: string): Promise<AccountData | null>;
}

export function createUpstreamClient(
  upstreamUrl: string,
  rpcTimeoutMs: number,
): UpstreamClient {
  async function rpcCall(method: string, params: unknown[]): Promise<unknown> {
    const body = JSON.stringify({ jsonrpc: "2.0", id: 1, method, params });
    let resp: Response;
    try {
      resp = await fetch(upstreamUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body,
        signal: AbortSignal.timeout(rpcTimeoutMs),
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

  async function getAccount(address: string): Promise<AccountData | null> {
    const result = (await rpcCall("getAccountInfo", [
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

  return { rpcCall, getAccount };
}
