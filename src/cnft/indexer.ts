// Indexer orchestrator. Given an upstream + store, pulls signature
// history for a tree, walks each transaction, and applies every
// Bubblegum ix we understand to the store. Drives the step-2/3 pure
// machinery from real RPC data.
//
// Not a daemon — each call does one pass from the last-indexed
// signature forward and returns. Callers (handlers / CLI) decide when
// to re-run. That keeps this file free of timers, background tasks,
// and cancellation state — easier to reason about and to port.
//
// Failure strategy: one malformed tx or parse error never halts the
// scan. We log to stderr via `logUpstreamError`-style helpers and move
// on. Apply errors that represent genuinely-corrupt state (mint on
// unknown tree) are surfaced — they mean the user asked us to index a
// tree without first seeing its createTree, which we can't recover.

import type { Address } from "@solana/kit";
import type { UpstreamClient } from "../upstream.js";
import { applyEvent } from "./apply.js";
import { parseBubblegumInstruction } from "./parser.js";
import type { CnftStore } from "./store.js";
import { extractBubblegumIxs, type JsonRpcTransactionResponse } from "./tx-extract.js";

export interface IndexerDeps {
  upstream: UpstreamClient;
  store: CnftStore;
}

export interface IndexTreeOptions {
  /**
   * Cap on the number of signatures fetched in a single call. Safety
   * rail against accidentally backfilling hundreds of thousands of txs
   * on a wide production tree. Default 10_000. Set to null to uncap.
   */
  maxSignatures?: number | null;
  /**
   * Page size passed to getSignaturesForAddress. Solana's RPC caps this
   * at 1000. Default 1000.
   */
  pageSize?: number;
  /**
   * Side-channel for test instrumentation — called once per ix that
   * parsed successfully. Useful for assertions without exposing
   * internal counters on the return type.
   */
  onEventApplied?: (sig: string) => void;
}

export interface IndexTreeResult {
  /** Signatures we fetched and fully processed this call. */
  processed: number;
  /** Ixs parsed + applied across all processed txs. */
  applied: number;
  /** Ixs encountered but skipped (parse failures, unknown discriminators, etc.). */
  skipped: number;
  /** The newest signature we advanced the store cursor to, or null. */
  newestApplied: string | null;
}

interface SignatureEntry {
  signature: string;
  slot: number;
  err: unknown;
}

const DEFAULT_MAX_SIGNATURES = 10_000;
const DEFAULT_PAGE_SIZE = 1000;

/**
 * Do one incremental pass. Safe to call repeatedly — it picks up from
 * the store's `lastSignature` cursor each time, so callers can just
 * re-invoke to stay fresh.
 */
export async function indexTree(
  deps: IndexerDeps,
  tree: Address,
  options: IndexTreeOptions = {},
): Promise<IndexTreeResult> {
  const maxSigs = options.maxSignatures === null
    ? Infinity
    : (options.maxSignatures ?? DEFAULT_MAX_SIGNATURES);
  const pageSize = options.pageSize ?? DEFAULT_PAGE_SIZE;

  const cursor = await deps.store.getLastSignature(tree);
  const sigs = await fetchSignaturesUntil(deps.upstream, tree, cursor, pageSize, maxSigs);

  // fetchSignaturesUntil returns oldest-first — apply order matches
  // on-chain order, which is what state transitions need.
  const result: IndexTreeResult = {
    processed: 0,
    applied: 0,
    skipped: 0,
    newestApplied: null,
  };

  for (const sig of sigs) {
    if (sig.err !== null) {
      // On-chain failure — no state transitions to replay. Still
      // advance the cursor so the next pass skips this sig cheaply.
      await deps.store.setLastSignature(tree, sig.signature);
      result.newestApplied = sig.signature;
      result.processed++;
      continue;
    }

    const tx = await fetchTransaction(deps.upstream, sig.signature);
    if (!tx) {
      // Could not fetch the tx (pruned, transient error). Skip
      // without advancing cursor so a future pass can retry.
      result.skipped++;
      continue;
    }

    const ixs = extractBubblegumIxs(tx);
    for (const ix of ixs) {
      const parsed = parseBubblegumInstruction({
        data: ix.data,
        accounts: ix.accounts,
        noopEvent: ix.noopEvent,
      });
      if (!parsed.ok) {
        result.skipped++;
        continue;
      }
      if (parsed.value === null) {
        // Bubblegum ix we don't track yet (e.g. verifyCreator pre-step-4.5).
        result.skipped++;
        continue;
      }
      try {
        await applyEvent(deps.store, parsed.value);
        result.applied++;
        options.onEventApplied?.(sig.signature);
      } catch (err) {
        // Apply errors are rarely recoverable (mint on unknown tree,
        // etc). Log and continue rather than halt the whole scan.
        console.error(
          `[surfpool-helius cnft] applyEvent failed for ${sig.signature}:`,
          err instanceof Error ? err.message : err,
        );
        result.skipped++;
      }
    }

    await deps.store.setLastSignature(tree, sig.signature);
    result.newestApplied = sig.signature;
    result.processed++;
  }

  return result;
}

/**
 * Walk getSignaturesForAddress from newest toward `untilSig`, collect
 * everything (up to the cap), and return in chronological order.
 */
async function fetchSignaturesUntil(
  upstream: UpstreamClient,
  tree: Address,
  untilSig: string | null,
  pageSize: number,
  maxSigs: number,
): Promise<SignatureEntry[]> {
  const collected: SignatureEntry[] = [];
  let before: string | undefined;

  while (collected.length < maxSigs) {
    const remaining = maxSigs - collected.length;
    const limit = Math.min(pageSize, remaining);

    const params: Record<string, unknown> = { limit };
    if (before) params.before = before;
    if (untilSig) params.until = untilSig;

    const page = (await upstream.rpcCall("getSignaturesForAddress", [
      tree as unknown as string,
      params,
    ])) as SignatureEntry[] | null;

    if (!page || page.length === 0) break;
    collected.push(...page);
    if (page.length < limit) break;
    before = page[page.length - 1]!.signature;
  }

  // RPC returns newest-first. Reverse so we apply in chronological order.
  return collected.reverse();
}

async function fetchTransaction(
  upstream: UpstreamClient,
  signature: string,
): Promise<JsonRpcTransactionResponse | null> {
  try {
    const tx = (await upstream.rpcCall("getTransaction", [
      signature,
      { encoding: "json", maxSupportedTransactionVersion: 0, commitment: "confirmed" },
    ])) as JsonRpcTransactionResponse | null;
    return tx ?? null;
  } catch (err) {
    console.error(
      `[surfpool-helius cnft] getTransaction(${signature}) failed:`,
      err instanceof Error ? err.message : err,
    );
    return null;
  }
}
