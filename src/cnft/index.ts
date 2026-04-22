// cNFT module surface. Step 3 adds the parser, the store, and the
// apply-event function. Indexer + DAS handler wiring land in subsequent
// steps.

export type {
  Creator,
  TreeInfo,
  TreeState,
  LeafSchemaV1,
  LeafRecord,
  MintMetadata,
  MerkleProof,
  ParseResult,
  CnftEvent,
  ParseError,
  NoopOverride,
} from "./types.js";

export {
  decodeLeafSchemaEvent,
  NOOP_PROGRAM_IDS,
  SPL_NOOP_PROGRAM_ID,
  MPL_NOOP_PROGRAM_ID,
} from "./leaf-event.js";
export type { LeafSchemaEventDecoded } from "./leaf-event.js";

export {
  keccak256,
  hashPair,
  emptyNode,
  hashLeafV1,
  hashMetadataArgsBytes,
  hashCreators,
} from "./hash.js";

export { computeProof, verifyProof } from "./proof.js";

export {
  parseBubblegumInstruction,
  BUBBLEGUM_PROGRAM_ADDRESS,
} from "./parser.js";

export type { CnftStore } from "./store.js";
export { createCnftMemoryStore } from "./store-memory.js";
export { applyEvent, deriveAssetId } from "./apply.js";

export {
  extractBubblegumIxs,
} from "./tx-extract.js";
export type {
  ExtractedIx,
  JsonRpcInstruction,
  JsonRpcInnerIxGroup,
  JsonRpcMeta,
  JsonRpcMessage,
  JsonRpcTransactionResponse,
} from "./tx-extract.js";

export { indexTree } from "./indexer.js";
export type { IndexerDeps, IndexTreeOptions, IndexTreeResult } from "./indexer.js";
