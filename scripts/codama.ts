// Generate a Kit-native client from a pinned Anchor/Shank IDL.
//
// Usage:
//   pnpm tsx scripts/codama.ts              # default: mpl-core
//   pnpm tsx scripts/codama.ts mpl-core
//   pnpm tsx scripts/codama.ts token-metadata
//
// Or via make:
//   make codegen                            # mpl-core
//   make codegen-token-metadata
//   make update-idl                         # refresh mpl-core IDL, then codegen
//   make update-idl-token-metadata
//
// Each registered program maps to a pinned IDL at `idls/<idl>.json` and a
// generated output directory at `src/generated/<out>/`. The provenance of
// the pinned IDL lives at `idls/<idl>.source.json`. Both files are
// committed so installs are reproducible without running codegen.
// `src/generated/` is the only place in src/ where you'll see
// auto-generated files — everything else is hand-maintained.
//
// Note: Codama's default output is a mini-package layout
// (<out>/package.json + <out>/src/generated/). We flatten it so our
// import paths stay shallow: src/generated/<out>/accounts/foo.ts
// instead of src/generated/<out>/src/generated/accounts/foo.ts.
// Relative imports within the generated code survive the move.

import { createFromRoot } from "codama";
import { rootNodeFromAnchor, type AnchorIdl } from "@codama/nodes-from-anchor";
import { renderVisitor } from "@codama/renderers-js";
import {
  readFileSync,
  rmSync,
  existsSync,
  mkdtempSync,
  cpSync,
  mkdirSync,
  writeFileSync,
  readdirSync,
  statSync,
} from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { tmpdir } from "node:os";

interface ProgramTarget {
  /** Filename under `idls/` without extension. */
  idl: string;
  /** Subdirectory under `src/generated/`. */
  out: string;
  /**
   * Codama subtrees to delete after render. Pruning drops dead code (and
   * its transitive deps — e.g. `programs/` pulls `@solana/program-client-core`)
   * so we only compile what we actually use.
   *
   * Default (account-reader targets): prune instructions/programs/errors.
   * Override for programs where we need the instruction decoders —
   * e.g. Bubblegum, where the whole cNFT indexer is ix-parser-driven.
   */
  pruneDirs?: string[];
}

const DEFAULT_PRUNE_DIRS = ["instructions", "programs", "errors"];

// Subtrees eligible for re-export from the per-target root index.ts.
// Order controls the export order in the emitted file.
const KNOWN_SUBTREES = ["accounts", "types", "instructions", "programs", "errors"] as const;

// Add new Codama targets here. Each entry makes `pnpm tsx scripts/codama.ts <key>`
// read `idls/<idl>.json` and emit to `src/generated/<out>/`.
const TARGETS: Record<string, ProgramTarget> = {
  "mpl-core": { idl: "mpl_core", out: "mpl-core" },
  "token-metadata": { idl: "token_metadata", out: "token-metadata" },
  // Bubblegum and spl-account-compression power cNFT indexing. We keep
  // `instructions/` for both because the indexer is driven by parsing
  // Bubblegum ixs and spl-account-compression CPIs.
  bubblegum: {
    idl: "bubblegum",
    out: "bubblegum",
    pruneDirs: ["errors"],
  },
  "spl-account-compression": {
    idl: "spl_account_compression",
    out: "spl-account-compression",
    pruneDirs: ["errors"],
  },
};

const programKey = process.argv[2] ?? "mpl-core";
const target = TARGETS[programKey];
if (!target) {
  const known = Object.keys(TARGETS).join(", ");
  throw new Error(
    `Unknown program target "${programKey}". Known: ${known}. Add an entry to TARGETS in scripts/codama.ts to register a new program.`,
  );
}

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..");

const idlPath = resolve(repoRoot, "idls", `${target.idl}.json`);
const finalOutputPath = resolve(repoRoot, "src", "generated", target.out);

const idlRaw = readFileSync(idlPath, "utf-8");
const idl = JSON.parse(idlRaw) as AnchorIdl;

console.log(`[codama] Target: ${programKey}`);
console.log(`[codama] IDL:    ${idlPath}`);
console.log(`[codama] Output: ${finalOutputPath}`);

// Render into a temporary directory first, then move just the useful
// subtree (src/generated/) into our target path. This gives us a flat
// layout without having to post-process Codama's output in place.
const scratch = mkdtempSync(resolve(tmpdir(), "surfpool-helius-codama-"));
try {
  const codama = createFromRoot(rootNodeFromAnchor(idl));
  await codama.accept(renderVisitor(scratch));

  const scratchInner = resolve(scratch, "src", "generated");
  if (!existsSync(scratchInner)) {
    throw new Error(
      `Expected generated output at ${scratchInner} — Codama layout changed?`,
    );
  }

  if (existsSync(finalOutputPath)) {
    rmSync(finalOutputPath, { recursive: true, force: true });
  }
  mkdirSync(finalOutputPath, { recursive: true });
  cpSync(scratchInner, finalOutputPath, { recursive: true });
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

// Prune subtrees we don't use. Per-target config — see ProgramTarget.
const pruneDirs = target.pruneDirs ?? DEFAULT_PRUNE_DIRS;
for (const dir of pruneDirs) {
  const path = resolve(finalOutputPath, dir);
  if (existsSync(path)) {
    rmSync(path, { recursive: true, force: true });
  }
}

// Rewrite the root index to re-export only the subtrees we kept. `types/`
// is always emitted by Codama; other subtrees may or may not exist
// depending on the IDL and the prune list.
const keptSubtrees = KNOWN_SUBTREES.filter((sub) =>
  existsSync(resolve(finalOutputPath, sub)),
);
if (existsSync(resolve(finalOutputPath, "types"))) {
  // Types always last so account/instruction re-exports take precedence
  // in case of collisions (Codama occasionally generates homonyms).
  keptSubtrees.sort((a, b) => (a === "types" ? 1 : b === "types" ? -1 : 0));
}
writeFileSync(
  resolve(finalOutputPath, "index.ts"),
  `// AUTOGENERATED — see scripts/codama.ts.
// Pruned subtrees for this target: ${pruneDirs.length ? pruneDirs.join(", ") : "(none)"}.

${keptSubtrees.map((sub) => `export * from "./${sub}/index.js";`).join("\n")}
`,
);

// Post-process imports. Codama emits TypeScript-style bare specifiers
// ("from '../types'", "from './assetV1'") which TSC with Bundler
// resolution accepts, but Node ESM requires explicit .js extensions and
// /index.js for directory imports. We rewrite every relative import to
// its resolved runtime path so `node dist/...` actually works.
function walkTs(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const p = resolve(dir, entry);
    const s = statSync(p);
    if (s.isDirectory()) {
      out.push(...walkTs(p));
    } else if (entry.endsWith(".ts")) {
      out.push(p);
    }
  }
  return out;
}

function normalizeImport(fileDir: string, spec: string): string {
  if (!spec.startsWith(".")) return spec;
  if (spec.endsWith(".js") || spec.endsWith(".json")) return spec;
  const abs = resolve(fileDir, spec);
  if (existsSync(`${abs}.ts`)) return `${spec}.js`;
  if (existsSync(abs) && statSync(abs).isDirectory()) {
    return `${spec}/index.js`;
  }
  return spec;
}

// Matches any relative specifier: "." / ".." / "./x" / "../y" / "./a/b"
// — i.e. starts with `.` and contains no protocol or bare package prefix.
const importRegex =
  /((?:from|import)\s*)(["'])(\.[^"']*)\2/g;

let rewrittenFiles = 0;
let rewrittenImports = 0;
for (const file of walkTs(finalOutputPath)) {
  const src = readFileSync(file, "utf-8");
  const fileDir = dirname(file);
  let changedHere = 0;
  const next = src.replace(importRegex, (_m, pre, quote, spec) => {
    const normalized = normalizeImport(fileDir, spec);
    if (normalized !== spec) changedHere++;
    return `${pre}${quote}${normalized}${quote}`;
  });
  if (changedHere > 0) {
    writeFileSync(file, next);
    rewrittenFiles++;
    rewrittenImports += changedHere;
  }
}

console.log(`[codama] Generated client written to ${finalOutputPath}`);
console.log(`[codama] Pruned: ${pruneDirs.join(", ")}`);
console.log(
  `[codama] Rewrote ${rewrittenImports} imports across ${rewrittenFiles} files for Node ESM`,
);
