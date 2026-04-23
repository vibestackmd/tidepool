/**
 * mint-and-query — prove the full tidepool loop works.
 *
 * 1. Point a UMI instance at the PROXY (not Surfpool directly). Every RPC call
 *    the Metaplex SDK makes — getLatestBlockhash, sendTransaction,
 *    confirmTransaction, getAccountInfo — flows through tidepool.
 * 2. Generate a fresh keypair and airdrop SOL on the local validator.
 * 3. Mint assets:
 *      (a) a plain MplCore asset (validates the base-header decode path)
 *      (b) an MplCore asset with a Royalties plugin (validates the plugin
 *          walker and DasAsset.creators population added in v0.3)
 *      (c) a legacy Metaplex Token Metadata master edition NFT with
 *          `printSupply: Limited(10)` (validates the Codama-generated
 *          decoder, mint-as-id routing in fetch.ts, and the
 *          getProgramAccounts holder scan — all v0.5.0)
 *      (d) a print edition of (c) via `printV1` — validates v0.5.1's
 *          Edition PDA side-effect indexing that populates
 *          getNftEditions.editions[]
 * 4. `sendAndConfirm` internally uses `signatureSubscribe`, which Surfpool
 *    doesn't support natively — the proxy's WS polyfill carries the weight.
 * 5. Query all four assets back via `getAsset`. `getNftEditions` is called
 *    twice on (c): once BEFORE (d) is fetched (asserts editions[] is
 *    empty) and once AFTER (asserts the print is in the list).
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
  TokenStandard,
  createNft,
  mplTokenMetadata,
  printSupply,
  printV1,
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
    name: "tidepool demo",
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
    name: "tidepool plugin demo",
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
    name: "tidepool legacy demo",
    uri: "https://example.com/demo.json",
    sellerFeeBasisPoints: percentAmount(5),
    creators: [
      { address: publicKey(payer.publicKey), share: 100, verified: true },
    ],
    printSupply: printSupply("Limited", [10]),
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

  // ── Initial getNftEditions on C (empty-before proof) ────────────────
  console.log("\n→ getNftEditions on C (before printing any)…");
  const editionsBefore = await rpc<{
    master_edition_address: string;
    supply: number;
    max_supply: number;
    total: number;
    editions: unknown[];
  }>("getNftEditions", { mint: mintC.publicKey.toString() });
  console.log(`  master_edition_address: ${editionsBefore.master_edition_address}`);
  console.log(`  supply:                 ${editionsBefore.supply}`);
  console.log(`  max_supply:             ${editionsBefore.max_supply}`);
  console.log(`  editions.length:        ${editionsBefore.editions.length}`);

  // ── Print edition 1 of C as asset D ─────────────────────────────────
  // v0.5.1 adds side-effect indexing: when fetch.ts sees an EditionV1
  // account while processing a mint-as-id lookup, it records the
  // parent + edition number in the CacheStore edition index. The
  // subsequent getNftEditions call on C should surface D.
  const mintD = generateSigner(umi);
  console.log(`\n→ printing edition 1 of C: ${mintD.publicKey}`);
  await printV1(umi, {
    masterTokenAccountOwner: umi.identity,
    masterEditionMint: mintC.publicKey,
    editionMint: mintD,
    editionNumber: 1,
    tokenStandard: TokenStandard.NonFungible,
  }).sendAndConfirm(umi);

  // ── Query D via getAsset (triggers edition-index side effect) ───────
  console.log("\n→ getAsset on D (print edition, by mint)…");
  const dasD = await rpc<DasAssetResponse>("getAsset", {
    id: mintD.publicKey.toString(),
  });
  console.log(`  id:        ${dasD.id}`);
  console.log(`  interface: ${dasD.interface}`);
  console.log(`  owner:     ${dasD.ownership.owner}`);

  // ── Re-query getNftEditions on C (indexed-after proof) ──────────────
  console.log("\n→ getNftEditions on C (after fetching D)…");
  const editionsAfter = await rpc<{
    master_edition_address: string;
    supply: number;
    max_supply: number;
    total: number;
    editions: Array<{ mint: string; edition: number; edition_address: string }>;
  }>("getNftEditions", { mint: mintC.publicKey.toString() });
  console.log(`  supply:          ${editionsAfter.supply}`);
  console.log(`  total:           ${editionsAfter.total}`);
  console.log(`  editions[0]:     ${JSON.stringify(editionsAfter.editions[0])}`);

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
    ["C name matches", dasC.content.metadata.name === "tidepool legacy demo"],
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

    // ── Initial getNftEditions (empty-before proof) ─────────────────
    [
      "editionsBefore.master_edition_address present",
      typeof editionsBefore.master_edition_address === "string" &&
        editionsBefore.master_edition_address.length > 0,
    ],
    ["editionsBefore supply is 0", editionsBefore.supply === 0],
    [
      "editionsBefore editions[] empty (LOCAL_INDEX, nothing seen yet)",
      editionsBefore.editions.length === 0,
    ],

    // ── Print edition D ─────────────────────────────────────────────
    ["D id matches print mint", dasD.id === mintD.publicKey.toString()],
    ["D interface is V1_NFT", dasD.interface === "V1_NFT"],
    ["D owner is payer", dasD.ownership.owner === payer.publicKey.toString()],

    // ── Post-print getNftEditions (indexed-after proof) ─────────────
    [
      "editionsAfter master_edition_address matches",
      editionsAfter.master_edition_address === editionsBefore.master_edition_address,
    ],
    ["editionsAfter supply is 1", editionsAfter.supply === 1],
    ["editionsAfter total is 1", editionsAfter.total === 1],
    [
      "editionsAfter.editions[0].mint matches D",
      editionsAfter.editions[0]?.mint === mintD.publicKey.toString(),
    ],
    [
      "editionsAfter.editions[0].edition is 1",
      editionsAfter.editions[0]?.edition === 1,
    ],

    // ── Cross-asset cache indexes ───────────────────────────────────
    [
      "getAssetsByCreator includes B and C",
      byCreatorIds.has(assetB.publicKey.toString()) &&
        byCreatorIds.has(mintC.publicKey.toString()),
    ],
    [
      "getAssetsByOwner includes A, B, C, and D",
      byOwnerIds.has(assetA.publicKey.toString()) &&
        byOwnerIds.has(assetB.publicKey.toString()) &&
        byOwnerIds.has(mintC.publicKey.toString()) &&
        byOwnerIds.has(mintD.publicKey.toString()),
    ],
    [
      "searchAssets(creatorAddress) includes B and C",
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
