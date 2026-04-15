/**
 * mint-and-query — prove the full surfpool-helius loop works.
 *
 * 1. Point a UMI instance at the PROXY (not Surfpool directly). Every RPC call
 *    the Metaplex SDK makes — getLatestBlockhash, sendTransaction,
 *    confirmTransaction, getAccountInfo — flows through surfpool-helius.
 * 2. Generate a fresh keypair and airdrop SOL on the local validator.
 * 3. Mint three assets:
 *      (a) a plain MplCore asset (validates the base-header decode path)
 *      (b) an MplCore asset with a Royalties plugin (validates the plugin
 *          walker and DasAsset.creators population added in v0.3)
 *      (c) a legacy Metaplex Token Metadata NFT (validates the
 *          Codama-generated decoder, the mint-as-id routing in fetch.ts,
 *          and the getTokenLargestAccounts owner resolution — all added
 *          in v0.5.0)
 * 4. `sendAndConfirm` internally uses `signatureSubscribe`, which Surfpool
 *    doesn't support natively — the proxy's WS polyfill carries the weight.
 * 5. Query all three assets back via `getAsset`. For (c), also call
 *    `getNftEditions` to prove the MasterEditionV2 decode path works.
 *    Finally, query `getAssetsByCreator` and `searchAssets` to confirm
 *    the local index picked up creators from both the MplCore Royalties
 *    plugin (b) and the Token Metadata creators array (c).
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
  createNft,
  mplTokenMetadata,
} from "@metaplex-foundation/mpl-token-metadata";
import {
  generateSigner,
  percentAmount,
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

  const umi = createUmi(PROXY_URL).use(mplCore()).use(mplTokenMetadata());

  const payer = generateSigner(umi);
  umi.use(signerIdentity(payer));
  console.log(`→ payer: ${payer.publicKey}`);

  console.log("→ airdropping 2 SOL…");
  await umi.rpc.airdrop(payer.publicKey, sol(2));

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

  // ── Mint asset C: legacy Metaplex Token Metadata NFT ────────────────
  // v0.5.0 adds a Codama-generated Token Metadata decoder. `createNft`
  // from mpl-token-metadata creates a MetadataV1 account (at a PDA
  // derived from the mint) plus a MasterEditionV2 account (at another
  // PDA). The mint itself is owned by SPL Token, not the Token Metadata
  // program — which is exactly the mint-as-id routing path fetch.ts
  // has to handle.
  const mintC = generateSigner(umi);
  console.log(
    `\n→ minting asset C (legacy Token Metadata NFT): ${mintC.publicKey}`,
  );
  await createNft(umi, {
    mint: mintC,
    name: "surfpool-helius legacy demo",
    uri: "https://example.com/demo.json",
    sellerFeeBasisPoints: percentAmount(5),
    creators: [
      { address: publicKey(payer.publicKey), share: 100, verified: true },
    ],
  }).sendAndConfirm(umi);

  // ── Query C via getAsset (mint-as-id routing) ───────────────────────
  // The id we pass is the mint, NOT the metadata PDA. fetch.ts detects
  // the SPL Token mint owner + 82-byte size and re-routes to the PDA.
  console.log("\n→ getAsset on C (by mint)…");
  const dasC = await rpc<DasAssetResponse>("getAsset", {
    id: mintC.publicKey.toString(),
  });
  console.log(`  id:        ${dasC.id}`);
  console.log(`  interface: ${dasC.interface}`);
  console.log(`  name:      ${dasC.content.metadata.name}`);
  console.log(`  owner:     ${dasC.ownership.owner}`);
  console.log(`  creators:  ${JSON.stringify(dasC.creators)}`);

  // ── Query C's master edition via getNftEditions ─────────────────────
  console.log("\n→ getNftEditions on C…");
  const editions = await rpc<{
    master_edition_address: string;
    supply: number;
    max_supply: number;
    editions: unknown[];
  }>("getNftEditions", { mint: mintC.publicKey.toString() });
  console.log(`  master_edition_address: ${editions.master_edition_address}`);
  console.log(`  supply:                 ${editions.supply}`);
  console.log(`  max_supply:             ${editions.max_supply}`);
  console.log(`  editions.length:        ${editions.editions.length}`);

  // ── Query both B and C via getAssetsByCreator ───────────────────────
  console.log("\n→ getAssetsByCreator(payer)…");
  const byCreator = await rpc<{ total: number; items: DasAssetResponse[] }>(
    "getAssetsByCreator",
    { creatorAddress: payer.publicKey.toString() },
  );
  console.log(`  total: ${byCreator.total}`);

  // ── Query all three via getAssetsByOwner ────────────────────────────
  // Proves owner resolution worked for C (the Token Metadata decoder
  // returns an empty owner, and fetch.ts fills it in via
  // getTokenLargestAccounts + a direct SPL Token account read).
  console.log("\n→ getAssetsByOwner(payer)…");
  const byOwner = await rpc<{ total: number; items: DasAssetResponse[] }>(
    "getAssetsByOwner",
    { ownerAddress: payer.publicKey.toString() },
  );
  console.log(`  total: ${byOwner.total}`);

  // ── Query B+C via searchAssets ──────────────────────────────────────
  console.log("\n→ searchAssets({ creatorAddress: payer })…");
  const search = await rpc<{ total: number; items: DasAssetResponse[] }>(
    "searchAssets",
    { creatorAddress: payer.publicKey.toString() },
  );
  console.log(`  total: ${search.total}`);

  // ── Assertions ──────────────────────────────────────────────────────
  const byCreatorIds = new Set(byCreator.items.map((a) => a.id));
  const byOwnerIds = new Set(byOwner.items.map((a) => a.id));
  const searchIds = new Set(search.items.map((a) => a.id));

  const assertions: Array<[string, boolean]> = [
    // ── MplCore asset A (plain) ─────────────────────────────────────
    ["A id matches", dasA.id === assetA.publicKey.toString()],
    ["A interface is MplCoreAsset", dasA.interface === "MplCoreAsset"],
    ["A owner is payer", dasA.ownership.owner === payer.publicKey.toString()],
    ["A has no creators (no plugins)", dasA.creators.length === 0],

    // ── MplCore asset B (with Royalties plugin) ─────────────────────
    ["B id matches", dasB.id === assetB.publicKey.toString()],
    ["B interface is MplCoreAsset", dasB.interface === "MplCoreAsset"],
    ["B owner is payer", dasB.ownership.owner === payer.publicKey.toString()],
    ["B has exactly 1 creator", dasB.creators.length === 1],
    [
      "B creator address is payer",
      dasB.creators[0]?.address === payer.publicKey.toString(),
    ],
    ["B creator share is 100", dasB.creators[0]?.share === 100],

    // ── Token Metadata legacy NFT asset C ───────────────────────────
    ["C id matches mint (mint-as-id routing)", dasC.id === mintC.publicKey.toString()],
    [
      "C interface is V1_NFT",
      dasC.interface === "V1_NFT",
    ],
    ["C name matches", dasC.content.metadata.name === "surfpool-helius legacy demo"],
    [
      "C owner resolved via getProgramAccounts holder scan",
      dasC.ownership.owner === payer.publicKey.toString(),
    ],
    ["C has 1 creator", dasC.creators.length === 1],
    [
      "C creator address is payer",
      dasC.creators[0]?.address === payer.publicKey.toString(),
    ],
    ["C creator share is 100", dasC.creators[0]?.share === 100],
    ["C creator is verified", dasC.creators[0]?.verified === true],

    // ── getNftEditions ──────────────────────────────────────────────
    [
      "getNftEditions master_edition_address present",
      typeof editions.master_edition_address === "string" &&
        editions.master_edition_address.length > 0,
    ],
    ["getNftEditions supply is 0", editions.supply === 0],
    [
      "getNftEditions editions[] empty (LOCAL_INDEX)",
      editions.editions.length === 0,
    ],

    // ── Cross-asset cache indexes ───────────────────────────────────
    [
      "getAssetsByCreator returns B and C (2)",
      byCreator.total === 2 &&
        byCreatorIds.has(assetB.publicKey.toString()) &&
        byCreatorIds.has(mintC.publicKey.toString()),
    ],
    [
      "getAssetsByOwner returns A, B, and C (3)",
      byOwner.total === 3 &&
        byOwnerIds.has(assetA.publicKey.toString()) &&
        byOwnerIds.has(assetB.publicKey.toString()) &&
        byOwnerIds.has(mintC.publicKey.toString()),
    ],
    [
      "searchAssets(creatorAddress) returns B and C (2)",
      search.total === 2 &&
        searchIds.has(assetB.publicKey.toString()) &&
        searchIds.has(mintC.publicKey.toString()),
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
