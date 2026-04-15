// helius.getTokenAccountsByOwnerV2 — cursor-paginated wrapper over
// standard getTokenAccountsByOwner. Same shape as getProgramAccountsV2:
// forward the base request, apply cursor logic to the result array.
//
// Helius's real implementation adds `changedSinceSlot` for incremental
// updates; we ignore it and document via the cursor helper's note.

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";
import { applyCursor, type CursorParams } from "./cursor.js";

interface GetTokenAccountsByOwnerV2Params extends CursorParams {
  owner?: string;
  mint?: string;
  programId?: string;
  encoding?: string;
  commitment?: string;
}

export const getTokenAccountsByOwnerV2: Handler = async (ctx, params, id) => {
  let owner: string | undefined;
  let opts: GetTokenAccountsByOwnerV2Params;

  if (Array.isArray(params)) {
    owner = params[0] as string;
    opts = (params[2] ?? params[1] ?? {}) as GetTokenAccountsByOwnerV2Params;
  } else {
    opts = (params ?? {}) as GetTokenAccountsByOwnerV2Params;
    owner = opts.owner;
  }

  if (!owner) {
    return {
      jsonrpc: "2.0",
      id,
      error: { code: -32602, message: "owner is required" },
    };
  }

  // Standard getTokenAccountsByOwner takes a mint OR a programId filter
  // as its second positional argument. Mirror that shape.
  const filter: Record<string, string> = {};
  if (opts.mint) filter.mint = opts.mint;
  else if (opts.programId) filter.programId = opts.programId;
  else {
    // Default to the SPL Token program if neither filter is given.
    filter.programId = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
  }

  const upstreamConfig: Record<string, unknown> = {};
  if (opts.encoding) upstreamConfig.encoding = opts.encoding;
  else upstreamConfig.encoding = "base64";
  if (opts.commitment) upstreamConfig.commitment = opts.commitment;

  const result = (await ctx.upstream.rpcCall("getTokenAccountsByOwner", [
    owner,
    filter,
    upstreamConfig,
  ])) as { context?: unknown; value?: unknown[] } | unknown[];

  // Response is usually wrapped in { context, value }. Normalize.
  const accounts = Array.isArray(result)
    ? (result as unknown[])
    : (result?.value ?? []);

  const cursored = applyCursor(accounts, opts);

  return jsonRpcResult(id, {
    tokenAccounts: cursored.items,
    paginationKey: cursored.paginationKey,
    ...(cursored.note ? { note: cursored.note } : {}),
  });
};
