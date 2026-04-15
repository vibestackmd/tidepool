// Namespaces module surface. Aggregates every namespace's handler map so
// the router can look up methods by name in one place. Adding a new
// namespace means importing its handler map and merging it here.

import type { Handler } from "../context.js";
import { dasHandlers } from "./das/index.js";

export const handlers: Record<string, Handler> = {
  ...dasHandlers,
};
