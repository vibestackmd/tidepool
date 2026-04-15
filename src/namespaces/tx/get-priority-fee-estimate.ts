// helius.tx.getPriorityFeeEstimate — percentile-based priority fee
// estimation. Real Helius computes this over their fleet-wide fee
// history; we approximate via the standard Solana RPC method
// `getRecentPrioritizationFees`, which returns up to 150 recent slot
// fee samples, and compute percentiles locally.
//
// Fidelity notes: on Surfpool (local validator with no competing
// traffic) priority fees are typically 0, so all percentiles return 0.
// This is correct behavior for local dev — no contention, no premium
// needed. When pointed at a congested mainnet upstream, the percentile
// approximation gets closer to real Helius's numbers but won't match
// exactly because Helius weighs their sample differently.
//
// Wire-level: this IS a real RPC method Helius serves on their endpoint,
// not a client-side SDK helper. Callers POST to our proxy and we return
// a Helius-shaped response.
//
// Params (all optional):
//   transaction       — base58-encoded tx. Currently ignored; we'd need
//                       to parse out the writable accounts to filter
//                       getRecentPrioritizationFees by them.
//   accountKeys       — addresses to narrow the sample to. Passed through.
//   options:
//     priorityLevel   — Min | Low | Medium | High | VeryHigh | UnsafeMax
//     includeAllPriorityFeeLevels — return all 6 percentiles instead
//                                   of just the selected one
//     lookbackSlots   — how far back to look (cap at 150, Solana's max)

import type { Handler } from "../../context.js";
import { jsonRpcResult } from "../../context.js";

type PriorityLevel = "Min" | "Low" | "Medium" | "High" | "VeryHigh" | "UnsafeMax";

interface GetPriorityFeeEstimateParams {
  transaction?: string;
  accountKeys?: string[];
  options?: {
    priorityLevel?: PriorityLevel;
    includeAllPriorityFeeLevels?: boolean;
    lookbackSlots?: number;
    recommended?: boolean;
  };
}

interface PrioritizationFeeSample {
  slot: number;
  prioritizationFee: number;
}

// Percentile of a sorted array. p in [0, 100]. Matches the "nearest rank"
// method — simple, stable, and sufficient for a local dev approximation.
function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  if (p <= 0) return sorted[0];
  if (p >= 100) return sorted[sorted.length - 1];
  const rank = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.max(0, Math.min(sorted.length - 1, rank))];
}

const LEVEL_PERCENTILES: Record<PriorityLevel, number> = {
  Min: 0,
  Low: 25,
  Medium: 50,
  High: 75,
  VeryHigh: 95,
  UnsafeMax: 100,
};

export const getPriorityFeeEstimate: Handler = async (ctx, params, id) => {
  const p = (params ?? {}) as GetPriorityFeeEstimateParams;
  const opts = p.options ?? {};
  const priorityLevel: PriorityLevel = opts.priorityLevel ?? "Medium";
  const includeAll = opts.includeAllPriorityFeeLevels === true;

  // Narrow the sample by accountKeys when provided. Solana's RPC method
  // takes an optional array of addresses and returns fees observed in
  // slots that touched those accounts — a better signal than the global
  // average when you know which accounts your tx will write.
  const rpcParams: unknown[] = p.accountKeys ? [p.accountKeys] : [];
  const samples = (await ctx.upstream.rpcCall(
    "getRecentPrioritizationFees",
    rpcParams,
  )) as PrioritizationFeeSample[] | null;

  const fees = Array.isArray(samples)
    ? samples.map((s) => s.prioritizationFee).sort((a, b) => a - b)
    : [];

  if (includeAll) {
    return jsonRpcResult(id, {
      priorityFeeLevels: {
        min: percentile(fees, LEVEL_PERCENTILES.Min),
        low: percentile(fees, LEVEL_PERCENTILES.Low),
        medium: percentile(fees, LEVEL_PERCENTILES.Medium),
        high: percentile(fees, LEVEL_PERCENTILES.High),
        veryHigh: percentile(fees, LEVEL_PERCENTILES.VeryHigh),
        unsafeMax: percentile(fees, LEVEL_PERCENTILES.UnsafeMax),
      },
    });
  }

  return jsonRpcResult(id, {
    priorityFeeEstimate: percentile(fees, LEVEL_PERCENTILES[priorityLevel]),
  });
};
