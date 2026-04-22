// Router. Resolves a JSON-RPC method name to a handler, or returns null
// if the method should fall through to the upstream passthrough path.
// The router is intentionally tiny — it has no business logic, just a
// lookup and a try/catch. All interesting work lives in the handlers.

import type { Handler, JsonRpcResponse, RequestContext } from "./context.js";
import { jsonRpcError } from "./context.js";
import { handlers as namespaceHandlers } from "./namespaces/index.js";
import { surfpoolHeliusInfo, surfpoolHeliusIndexTree } from "./compat/index.js";

const handlers: Record<string, Handler> = {
  ...namespaceHandlers,
  surfpoolHeliusInfo,
  surfpoolHeliusIndexTree,
};

export function findHandler(method: string): Handler | null {
  return handlers[method] ?? null;
}

export async function dispatch(
  ctx: RequestContext,
  method: string,
  params: unknown,
  id: unknown,
): Promise<JsonRpcResponse | null> {
  const handler = findHandler(method);
  if (!handler) return null;
  try {
    return await handler(ctx, params, id);
  } catch (err) {
    console.error(`[surfpool-helius] Error handling ${method}:`, err);
    return jsonRpcError(id, -32000, String(err));
  }
}
