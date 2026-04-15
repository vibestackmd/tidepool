// surfpoolHeliusInfo — a custom JSON-RPC method that returns the
// compatibility manifest. Callers can introspect which methods are
// implemented and at what fidelity before running against the proxy.
// Real Helius doesn't ship anything like this; it's a feature of the
// local-dev mirror, not a mirror of a Helius feature.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import type { Handler } from "../context.js";
import { jsonRpcResult } from "../context.js";
import { manifest, summarize } from "./manifest.js";

// Load the version from package.json at module init. The file sits two
// directories above dist/compat/info.js once compiled, or above
// src/compat/info.ts when running via tsx — both paths work because
// we resolve relative to this file's URL.
function loadVersion(): string {
  try {
    const here = dirname(fileURLToPath(import.meta.url));
    // dist/compat/info.js → ../../package.json
    // src/compat/info.ts  → ../../package.json (tsx preserves layout)
    const pkgPath = join(here, "..", "..", "package.json");
    const raw = readFileSync(pkgPath, "utf-8");
    const pkg = JSON.parse(raw) as { version?: string };
    return pkg.version ?? "unknown";
  } catch {
    return "unknown";
  }
}

const VERSION = loadVersion();

export const surfpoolHeliusInfo: Handler = async (_ctx, _params, id) => {
  return jsonRpcResult(id, {
    name: "surfpool-helius",
    version: VERSION,
    methods: manifest,
    summary: summarize(manifest),
  });
};
