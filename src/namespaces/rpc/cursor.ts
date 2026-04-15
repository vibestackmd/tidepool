// Shared cursor-pagination helper for Helius V2 RPC methods. Helius's
// V2 endpoints (getProgramAccountsV2, getTokenAccountsByOwnerV2, etc.)
// wrap standard Solana RPC results with cursor tokens so clients can
// iterate through large datasets without offset drift.
//
// Implementation: cursors are just base64-encoded "start offset"
// integers. Simple, stateless, and deterministic — same input always
// produces the same cursor. A fancier system would encode the sort
// order + a unique key per entry, but this is a local-dev mirror, not
// a production indexer.
//
// Note on `changedSinceSlot`: Helius's real V2 methods accept this
// parameter to return only accounts modified after a given slot. We
// don't track per-account slot metadata locally, so this filter is
// silently ignored — the response returns all matching accounts with
// a `note` field indicating the limitation.

const DEFAULT_LIMIT = 1000;
const MAX_LIMIT = 10_000;

export interface CursorParams {
  limit?: number;
  cursor?: string;
  changedSinceSlot?: number;
}

export interface CursoredResult<T> {
  items: T[];
  paginationKey: string | null;
  note?: string;
}

function encodeCursor(offset: number): string {
  return Buffer.from(String(offset), "utf-8").toString("base64");
}

function decodeCursor(cursor: string | undefined): number {
  if (!cursor) return 0;
  try {
    const decoded = Buffer.from(cursor, "base64").toString("utf-8");
    const n = parseInt(decoded, 10);
    return Number.isFinite(n) && n >= 0 ? n : 0;
  } catch {
    return 0;
  }
}

export function applyCursor<T>(
  items: T[],
  params: CursorParams,
): CursoredResult<T> {
  const start = decodeCursor(params.cursor);
  const requested = params.limit ?? DEFAULT_LIMIT;
  const limit = Math.max(1, Math.min(MAX_LIMIT, requested));
  const end = start + limit;
  const slice = items.slice(start, end);
  const paginationKey = end < items.length ? encodeCursor(end) : null;

  const result: CursoredResult<T> = { items: slice, paginationKey };

  if (params.changedSinceSlot !== undefined) {
    result.note =
      "changedSinceSlot is not supported by surfpool-helius — all matching accounts returned. This is a local-dev mirror without per-account slot tracking.";
  }

  return result;
}
