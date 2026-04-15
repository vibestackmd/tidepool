/**
 * Pluggable account decoder interface.
 *
 * A decoder turns raw Solana account bytes into a DAS-shaped asset. Ship your own
 * decoder to add support for any program — MplCore, Token-2022, legacy Token
 * Metadata, or custom NFT standards.
 *
 * The proxy picks a decoder by matching the account's `owner` program ID to the
 * decoder's `programId`. The first matching decoder wins.
 */

export interface DasAsset {
  id: string;
  interface: string;
  content: {
    $schema: string;
    json_uri: string;
    metadata: {
      name: string;
      symbol: string;
      description: string;
    };
    links: {
      image: string | null;
      animation_url: string | null;
    };
    files: Array<{ uri: string; mime: string }>;
  };
  authorities: Array<{ address: string; scopes: string[] }>;
  ownership: {
    frozen: boolean;
    delegated: boolean;
    ownership_model: string;
    owner: string;
  };
  grouping: Array<{ group_key: string; group_value: string }>;
  mutable: boolean;
  burnt: boolean;
}

export interface AccountDecoder {
  /** The program ID this decoder handles. Must match the account's `owner` field. */
  readonly programId: string;

  /** Human-readable name — used for the `interface` field on the DAS asset and for logs. */
  readonly name: string;

  /**
   * Decode raw account bytes into a DAS asset. Return null if the data doesn't
   * match this decoder (e.g. wrong account variant within the same program).
   */
  decode(pubkey: string, data: Uint8Array): Promise<DasAsset | null>;
}

export { mplCoreDecoder } from "./mpl-core.js";
