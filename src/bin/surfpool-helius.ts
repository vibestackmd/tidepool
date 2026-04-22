#!/usr/bin/env node
// CLI entry point. Parses flags and environment variables, then calls
// createProxy. Everything else is library code — this file is thin on
// purpose.

import { createProxy } from "../server/index.js";

interface CliArgs {
  port?: number;
  upstreamUrl?: string;
  upstreamWsUrl?: string;
  indexTrees: string[];
}

function parseArgs(argv: string[]): CliArgs {
  const args: CliArgs = { indexTrees: [] };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const next = argv[i + 1];
    if ((arg === "--port" || arg === "-p") && next) {
      args.port = parseInt(next, 10);
      i++;
    } else if (arg === "--upstream" && next) {
      args.upstreamUrl = next;
      i++;
    } else if (arg === "--upstream-ws" && next) {
      args.upstreamWsUrl = next;
      i++;
    } else if (arg === "--index-tree" && next) {
      args.indexTrees.push(next);
      i++;
    } else if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    }
  }
  return args;
}

function printHelp() {
  console.log(`surfpool-helius — run Helius DAS locally on Surfpool

Usage:
  surfpool-helius [options]

Options:
  -p, --port <port>          HTTP port to listen on (default: 8897)
      --upstream <url>       Upstream Solana RPC URL (default: http://127.0.0.1:8899)
      --upstream-ws <url>    Upstream Solana WebSocket URL (default: derived from --upstream host on port 8900)
      --index-tree <pubkey>  Bubblegum merkle tree to backfill on startup (repeatable)
  -h, --help                 Show this help

Environment:
  SURFPOOL_HELIUS_PORT
  SURFPOOL_HELIUS_UPSTREAM_URL
  SURFPOOL_HELIUS_UPSTREAM_WS_URL
  SURFPOOL_HELIUS_INDEX_TREES   comma-separated list of tree pubkeys

The WebSocket server runs on port + 1 (default: 8898) — web3.js auto-derives
the WS port as HTTP + 1 for localhost, so point your app at http://localhost:8897
and confirmTransaction() just works.
`);
}

const envPort = process.env.SURFPOOL_HELIUS_PORT;
const envUpstream = process.env.SURFPOOL_HELIUS_UPSTREAM_URL;
const envUpstreamWs = process.env.SURFPOOL_HELIUS_UPSTREAM_WS_URL;
const envIndexTrees = process.env.SURFPOOL_HELIUS_INDEX_TREES;

const cli = parseArgs(process.argv.slice(2));

const envIndexTreeList = envIndexTrees
  ? envIndexTrees.split(",").map((s) => s.trim()).filter((s) => s.length > 0)
  : [];
const indexTrees = [...cli.indexTrees, ...envIndexTreeList];

const options = {
  port: cli.port ?? (envPort ? parseInt(envPort, 10) : undefined),
  upstreamUrl: cli.upstreamUrl ?? envUpstream,
  upstreamWsUrl: cli.upstreamWsUrl ?? envUpstreamWs,
  indexTrees: indexTrees.length > 0 ? indexTrees : undefined,
};

createProxy(options).catch((err: Error) => {
  console.error(`[surfpool-helius] Failed to start: ${err.message}`);
  process.exit(1);
});
