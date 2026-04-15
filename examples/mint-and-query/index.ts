/**
 * mint-and-query — prove the full surfpool-helius loop works.
 *
 * 1. Point a UMI instance at the PROXY (not Surfpool directly). Every RPC call
 *    the Metaplex SDK makes — getLatestBlockhash, sendTransaction,
 *    confirmTransaction, getAccountInfo — flows through surfpool-helius.
 * 2. Generate a fresh keypair and airdrop SOL on the local validator.
 * 3. Mint two MplCore assets:
 *      (a) a plain asset (validates the base-header decode path)
 *      (b) an asset with a Royalties plugin (validates the plugin walker
 *          and DasAsset.creators population added in v0.3)
 * 4. `sendAndConfirm` internally uses `signatureSubscribe`, which Surfpool
 *    doesn't support natively — the proxy's WS polyfill carries the weight.
 * 5. Query both assets back via `getAsset`. Then query `getAssetsByCreator`
 *    and `searchAssets` to confirm the local index picked up the creator
 *    address from the Royalties plugin on asset (b).
 *
 * Prereqs: `pnpm install` in this folder, Surfpool running (`make up` in the
 * repo root), and the proxy running (`make dev`).
 */

import {
  create,
  fetchAssetV1,
  mplCore,
  ruleSet,
} from "@metaplex-foundation/mpl-core";
import {
  generateSigner,
  publicKey,
  signerIdentity,
  sol,
} from "@metaplex-foundation/umi";
import { createUmi } from "@metaplex-foundation/umi-bundle-defaults";

const PROXY_URL = process.env.PROXY_URL ?? "http://127.0.0.1:8897";

interface DasAssetResponse {
  id: string;
  interface: string;
  content: { metadata: { name: string }; json_uri: string };
  ownership: { owner: string };
  creators: Array<{ address: string; share: number; verified: boolean }>;
}

async function rpc<T>(method: string, params: unknown): Promise<T> {
  const resp = await fetch(PROXY_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const json = (await resp.json()) as { result?: T; error?: unknown };
  if (json.error || json.result === undefined) {
    throw new Error(
      `${method} failed: ${JSON.stringify(json.error ?? "no result")}`,
    );
  }
  return json.result;
}

async function main() {
  console.log(`→ proxy: ${PROXY_URL}`);

  const umi = createUmi(PROXY_URL).use(mplCore());

  const payer = generateSigner(umi);
  umi.use(signerIdentity(payer));
  console.log(`→ payer: ${payer.publicKey}`);

  console.log("→ airdropping 1 SOL…");
  await umi.rpc.airdrop(payer.publicKey, sol(1));

  // ── Mint asset A: no plugins ────────────────────────────────────────
  const assetA = generateSigner(umi);
  console.log(`→ minting asset A (no plugins): ${assetA.publicKey}`);
  await create(umi, {
    asset: assetA,
    name: "surfpool-helius demo",
    uri: "https://example.com/demo.json",
  }).sendAndConfirm(umi);

  const fetchedA = await fetchAssetV1(umi, publicKey(assetA.publicKey));
  console.log(`→ on-chain name: ${fetchedA.name}`);

  // ── Mint asset B: with a Royalties plugin ───────────────────────────
  // Royalties carries a creators[] list with address + percentage. The
  // plugin walker in v0.3 reads this and populates DasAsset.creators.
  const assetB = generateSigner(umi);
  console.log(`→ minting asset B (with Royalties plugin): ${assetB.publicKey}`);
  await create(umi, {
    asset: assetB,
    name: "surfpool-helius plugin demo",
    uri: "https://example.com/demo.json",
    plugins: [
      {
        type: "Royalties",
        basisPoints: 500,
        creators: [
          { address: publicKey(payer.publicKey), percentage: 100 },
        ],
        ruleSet: ruleSet("None"),
      },
    ],
  }).sendAndConfirm(umi);

  // ── Query A via getAsset ────────────────────────────────────────────
  console.log("\n→ getAsset on A…");
  const dasA = await rpc<DasAssetResponse>("getAsset", {
    id: assetA.publicKey.toString(),
  });
  console.log(`  id:       ${dasA.id}`);
  console.log(`  name:     ${dasA.content.metadata.name}`);
  console.log(`  owner:    ${dasA.ownership.owner}`);
  console.log(`  creators: ${JSON.stringify(dasA.creators)}`);

  // ── Query B via getAsset ────────────────────────────────────────────
  console.log("\n→ getAsset on B…");
  const dasB = await rpc<DasAssetResponse>("getAsset", {
    id: assetB.publicKey.toString(),
  });
  console.log(`  id:       ${dasB.id}`);
  console.log(`  name:     ${dasB.content.metadata.name}`);
  console.log(`  owner:    ${dasB.ownership.owner}`);
  console.log(`  creators: ${JSON.stringify(dasB.creators)}`);

  // ── Query B via getAssetsByCreator ──────────────────────────────────
  console.log("\n→ getAssetsByCreator(payer)…");
  const byCreator = await rpc<{ total: number; items: DasAssetResponse[] }>(
    "getAssetsByCreator",
    { creatorAddress: payer.publicKey.toString() },
  );
  console.log(`  total: ${byCreator.total}`);

  // ── Query B via searchAssets ────────────────────────────────────────
  console.log("\n→ searchAssets({ creatorAddress: payer })…");
  const search = await rpc<{ total: number; items: DasAssetResponse[] }>(
    "searchAssets",
    { creatorAddress: payer.publicKey.toString() },
  );
  console.log(`  total: ${search.total}`);

  // ── Assertions ──────────────────────────────────────────────────────
  const assertions: Array<[string, boolean]> = [
    ["A id matches", dasA.id === assetA.publicKey.toString()],
    ["A interface is MplCoreAsset", dasA.interface === "MplCoreAsset"],
    ["A owner is payer", dasA.ownership.owner === payer.publicKey.toString()],
    ["A has no creators (no plugins)", dasA.creators.length === 0],

    ["B id matches", dasB.id === assetB.publicKey.toString()],
    ["B interface is MplCoreAsset", dasB.interface === "MplCoreAsset"],
    ["B owner is payer", dasB.ownership.owner === payer.publicKey.toString()],
    ["B has exactly 1 creator", dasB.creators.length === 1],
    [
      "B creator address is payer",
      dasB.creators[0]?.address === payer.publicKey.toString(),
    ],
    ["B creator share is 100", dasB.creators[0]?.share === 100],

    ["getAssetsByCreator returns 1", byCreator.total === 1],
    [
      "getAssetsByCreator returns asset B",
      byCreator.items[0]?.id === assetB.publicKey.toString(),
    ],

    ["searchAssets(creatorAddress) returns 1", search.total === 1],
    [
      "searchAssets returns asset B",
      search.items[0]?.id === assetB.publicKey.toString(),
    ],
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

  console.log(
    "\n✔ round-trip succeeded — mint (plain + with plugins), confirm, fetch, and DAS queries all went through the proxy.",
  );
}

main().catch((err: Error) => {
  console.error("✖ example failed:", err.message);
  process.exit(1);
});
