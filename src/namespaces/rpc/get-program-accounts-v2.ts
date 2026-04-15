// helius.getProgramAccountsV2 — Helius's cursor-paginated variant of
// standard getProgramAccounts. For a local-dev mirror against Surfpool
// we pass the request straight through to the upstream's regular
// getProgramAccounts method, then wrap the result with cursor logic.
//
// This works well for programs with small-to-medium account counts
// (your own deployed program during testing). For programs with
// millions of accounts, the underlying getProgramAccounts call will
// time out — same failure mode as real Solana RPC. Real Helius sidesteps
// this via a dedicated indexer; we don't, and there's no clean local
// equivalent.
//
// See cursor.ts for the changedSinceSlot caveat.

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";
import { applyCursor, type CursorParams } from "./cursor.js";

interface GetProgramAccountsV2Params extends CursorParams {
  programId?: string;
  // Standard getProgramAccounts options (encoding, filters, etc.) are
  // forwarded unchanged to the upstream call.
  filters?: unknown[];
  encoding?: string;
  commitment?: string;
  dataSlice?: unknown;
  withContext?: boolean;
}

export const getProgramAccountsV2: Handler = async (ctx, params, id) => {
  // Positional form: [programId, { config }] — matches Helius docs.
  // Named form: { programId, filters, ... } — matches helius-sdk's
  // object-style call. Accept both.
  let programId: string | undefined;
  let opts: GetProgramAccountsV2Params;

  if (Array.isArray(params)) {
    programId = params[0] as string;
    opts = (params[1] ?? {}) as GetProgramAccountsV2Params;
  } else {
    opts = (params ?? {}) as GetProgramAccountsV2Params;
    programId = opts.programId;
  }

  if (!programId) {
    return {
      jsonrpc: "2.0",
      id,
      error: { code: -32602, message: "programId is required" },
    };
  }

  // Strip pagination + V2-specific fields before forwarding to the
  // standard RPC — Surfpool's getProgramAccounts doesn't know what
  // `cursor` or `changedSinceSlot` mean.
  const upstreamConfig: Record<string, unknown> = {};
  if (opts.filters) upstreamConfig.filters = opts.filters;
  if (opts.encoding) upstreamConfig.encoding = opts.encoding;
  if (opts.commitment) upstreamConfig.commitment = opts.commitment;
  if (opts.dataSlice) upstreamConfig.dataSlice = opts.dataSlice;
  if (opts.withContext) upstreamConfig.withContext = opts.withContext;

  const result = await ctx.upstream.rpcCall("getProgramAccounts", [
    programId,
    upstreamConfig,
  ]);

  // Standard getProgramAccounts returns either an array (default) or
  // { context, value } (withContext=true). Normalize to an array for
  // cursor application, then re-wrap if needed.
  const accounts = Array.isArray(result)
    ? (result as unknown[])
    : ((result as { value?: unknown[] })?.value ?? []);

  const cursored = applyCursor(accounts, opts);

  return jsonRpcResult(id, {
    programAccounts: cursored.items,
    paginationKey: cursored.paginationKey,
    ...(cursored.note ? { note: cursored.note } : {}),
  });
};
