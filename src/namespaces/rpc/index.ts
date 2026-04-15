// Helius-flavored V2 RPC namespace. These are standard Solana RPC
// methods re-wrapped with cursor pagination and (in real Helius)
// `changedSinceSlot` for incremental updates. We implement the cursor
// wrapping; incremental update filtering is documented as unsupported.
//
// Companion auto-paginators (getAllProgramAccounts, getAllTokenAccountsByOwner)
// are SDK_WRAPPER methods — they loop over the V2 methods in the caller's
// SDK and don't need a handler here.

import type { Handler } from "../../context.js";
import { getProgramAccountsV2 } from "./get-program-accounts-v2.js";
import { getTokenAccountsByOwnerV2 } from "./get-token-accounts-by-owner-v2.js";

export const rpcHandlers: Record<string, Handler> = {
  getProgramAccountsV2,
  getTokenAccountsByOwnerV2,
};
