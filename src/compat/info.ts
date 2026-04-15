// surfpoolHeliusInfo — a custom JSON-RPC method that returns the
// compatibility manifest. Callers can introspect which methods are
// implemented and at what fidelity before running against the proxy.
// Real Helius doesn't ship anything like this; it's a feature of the
// local-dev mirror, not a mirror of a Helius feature.

import type { Handler } from "../context.js";
import { jsonRpcResult } from "../context.js";
import { manifest, summarize } from "./manifest.js";

// package.json is loaded lazily via a small helper so the info handler
// doesn't need top-level file IO. Version is injected by the build.
const VERSION = "0.1.1";

export const surfpoolHeliusInfo: Handler = async (_ctx, _params, id) => {
  return jsonRpcResult(id, {
    name: "surfpool-helius",
    version: VERSION,
    methods: manifest,
    summary: summarize(manifest),
  });
};
