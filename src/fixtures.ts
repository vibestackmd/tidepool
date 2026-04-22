// Canned UpstreamClient for tests and for consumers who don't want real
// network traffic from their mock layer. Accepts a map of account data and
// a map of RPC responses — no fetch, no timeouts, fully deterministic.

import type { AccountData, UpstreamClient } from "./upstream.js";

export interface FixtureUpstreamOptions {
  /**
   * Map of Solana address → canned account data. `getAccount(address)`
   * returns the entry, or null when the address is absent.
   */
  accounts?: Record<string, AccountData | null>;
  /**
   * Map of JSON-RPC method name → result producer. The producer is called
   * with the params array and returns the raw `result` field (not a full
   * JSON-RPC envelope). Throw from the producer to surface an upstream
   * error the same way `createUpstreamClient` would.
   */
  rpcResponses?: Record<string, (params: unknown[]) => unknown | Promise<unknown>>;
}

export function createFixtureUpstream(
  options: FixtureUpstreamOptions = {},
): UpstreamClient {
  const accounts = options.accounts ?? {};
  const rpcResponses = options.rpcResponses ?? {};

  async function rpcCall(method: string, params: unknown[]): Promise<unknown> {
    // getAccountInfo is the one call that has a natural default: derive it
    // from the `accounts` fixture so consumers only have to populate one
    // map for the common case.
    if (method === "getAccountInfo" && !rpcResponses[method]) {
      const address = params[0] as string;
      const account = accounts[address] ?? null;
      if (!account) return { context: { slot: 0 }, value: null };
      return {
        context: { slot: 0 },
        value: {
          data: [Buffer.from(account.data).toString("base64"), "base64"],
          owner: account.owner,
          lamports: account.lamports,
          executable: false,
          rentEpoch: 0,
        },
      };
    }

    const producer = rpcResponses[method];
    if (!producer) {
      throw new Error(
        `createFixtureUpstream: no fixture for RPC method "${method}"`,
      );
    }
    return producer(params);
  }

  async function getAccount(address: string): Promise<AccountData | null> {
    if (address in accounts) return accounts[address];
    return null;
  }

  return { rpcCall, getAccount };
}
