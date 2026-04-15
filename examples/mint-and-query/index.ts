/**
 * mint-and-query — prove the full surfpool-helius loop works.
 *
 * 1. Point a UMI instance at the PROXY (not Surfpool directly). Every RPC call
 *    the Metaplex SDK makes — getLatestBlockhash, sendTransaction,
 *    confirmTransaction, getAccountInfo — flows through surfpool-helius.
 * 2. Generate a fresh keypair and airdrop SOL on the local validator.
 * 3. Mint an MplCore asset via the real Metaplex `create` instruction.
 *    `sendAndConfirm` internally uses `signatureSubscribe`, which Surfpool
 *    doesn't support natively — the proxy's WS polyfill carries the weight.
 * 4. Query the asset back via `getAsset`. The proxy fetches the raw account
 *    from Surfpool and runs it through the MplCore decoder.
 * 5. Assert the round-trip worked.
 *
 * Prereqs: `pnpm install` in this folder, Surfpool running (`make up` in the
 * repo root), and the proxy running (`make dev`).
 */

import {
  create,
  fetchAssetV1,
  mplCore,
} from "@metaplex-foundation/mpl-core";
import {
  generateSigner,
  publicKey,
  signerIdentity,
  sol,
} from "@metaplex-foundation/umi";
import { createUmi } from "@metaplex-foundation/umi-bundle-defaults";

const PROXY_URL = process.env.PROXY_URL ?? "http://127.0.0.1:8897";

async function main() {
  console.log(`→ proxy: ${PROXY_URL}`);

  // Point UMI at the proxy. Every RPC and WS call goes through surfpool-helius.
  const umi = createUmi(PROXY_URL).use(mplCore());

  // Fresh keypair for this run — no persistent state needed.
  const payer = generateSigner(umi);
  umi.use(signerIdentity(payer));
  console.log(`→ payer: ${payer.publicKey}`);

  // Airdrop — forwarded to Surfpool via the proxy.
  console.log("→ airdropping 1 SOL…");
  await umi.rpc.airdrop(payer.publicKey, sol(1));

  // Mint an MplCore asset.
  const asset = generateSigner(umi);
  console.log(`→ minting asset: ${asset.publicKey}`);
  await create(umi, {
    asset,
    name: "surfpool-helius demo",
    uri: "https://example.com/demo.json",
  }).sendAndConfirm(umi);

  // Confirm the asset exists on-chain via UMI (hits the proxy's passthrough).
  const fetched = await fetchAssetV1(umi, publicKey(asset.publicKey));
  console.log(`→ on-chain name: ${fetched.name}`);

  // Now query it through surfpool-helius's DAS endpoint.
  console.log("→ calling getAsset via the proxy…");
  const resp = await fetch(PROXY_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "getAsset",
      params: { id: asset.publicKey.toString() },
    }),
  });
  const dasResponse = (await resp.json()) as {
    result?: {
      id: string;
      interface: string;
      content: { metadata: { name: string }; json_uri: string };
      ownership: { owner: string };
    };
    error?: unknown;
  };

  if (dasResponse.error || !dasResponse.result) {
    console.error("✖ getAsset failed:", dasResponse.error ?? "no result");
    process.exit(1);
  }

  const das = dasResponse.result;
  console.log("\n✔ DAS response:");
  console.log(`  id:        ${das.id}`);
  console.log(`  interface: ${das.interface}`);
  console.log(`  name:      ${das.content.metadata.name}`);
  console.log(`  json_uri:  ${das.content.json_uri}`);
  console.log(`  owner:     ${das.ownership.owner}`);

  // Sanity checks.
  const assertions: Array<[string, boolean]> = [
    ["id matches", das.id === asset.publicKey.toString()],
    ["interface is MplCoreAsset", das.interface === "MplCoreAsset"],
    ["name matches", das.content.metadata.name === "surfpool-helius demo"],
    ["owner is payer", das.ownership.owner === payer.publicKey.toString()],
  ];

  console.log("\n→ assertions:");
  let allPassed = true;
  for (const [label, ok] of assertions) {
    console.log(`  ${ok ? "✔" : "✖"} ${label}`);
    if (!ok) allPassed = false;
  }

  if (!allPassed) {
    console.error("\n✖ one or more assertions failed");
    process.exit(1);
  }

  console.log("\n✔ round-trip succeeded — mint, confirm, fetch, and DAS query all went through the proxy.");
}

main().catch((err: Error) => {
  console.error("✖ example failed:", err.message);
  process.exit(1);
});
