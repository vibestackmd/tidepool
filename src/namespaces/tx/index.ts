// tx namespace surface. v0.4 ships getPriorityFeeEstimate — the only
// wire-level RPC method in helius.tx. The rest of helius.tx.* is SDK
// composition helpers that run in the caller's app and make standard
// wire-level RPC calls under the hood; they work transparently against
// this proxy without any handler here. See the manifest entries tagged
// SDK_WRAPPER for the full list.

import type { Handler } from "../../context.js";
import { getPriorityFeeEstimate } from "./get-priority-fee-estimate.js";

export const txHandlers: Record<string, Handler> = {
  getPriorityFeeEstimate,
};
