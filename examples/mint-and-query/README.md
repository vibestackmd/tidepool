# mint-and-query

End-to-end proof that tidepool works. This example:

1. Points [Metaplex UMI](https://developers.metaplex.com/umi) at the proxy (not Surfpool directly) so every RPC and WS call flows through tidepool.
2. Generates a fresh keypair and airdrops SOL on the local validator.
3. Mints an MplCore asset via the real Metaplex `create` instruction.
4. Waits for confirmation — `sendAndConfirm` uses `signatureSubscribe`. Tidepool reverse-proxies the WS connection to Surfpool's native subscription endpoint, so client code sees one URL while the actual subscription work happens upstream.
5. Queries the asset back via `getAsset` — handled by the proxy's MplCore decoder.
6. Asserts that the DAS response round-trips correctly (id, interface, name, owner).

If every step passes, the whole loop — transaction sending, WS-proxied subscription, account decoding — is verified end to end.

## Run it

You need Surfpool and tidepool running. From the repo root:

```bash
# Start Surfpool (or use the bundled docker compose: `docker compose up -d`)
surfpool start

# In another terminal, start the proxy
tidepool start --upstream http://127.0.0.1:8899
```

Then run the example:

```bash
cd examples/mint-and-query
pnpm install
pnpm start
```

Expected output:

```
→ proxy: http://127.0.0.1:8897
→ payer: <fresh pubkey>
→ airdropping 1 SOL…
→ minting asset: <fresh pubkey>
→ on-chain name: tidepool demo
→ calling getAsset via the proxy…

✔ DAS response:
  id:        <asset pubkey>
  interface: MplCoreAsset
  name:      tidepool demo
  json_uri:  https://example.com/demo.json
  owner:     <payer pubkey>

→ assertions:
  ✔ id matches
  ✔ interface is MplCoreAsset
  ✔ name matches
  ✔ owner is payer

✔ round-trip succeeded — mint, confirm, fetch, and DAS query all went through the proxy.
```

## Point at a different port

If your proxy is on a non-default port:

```bash
PROXY_URL=http://127.0.0.1:8796 pnpm start
```
