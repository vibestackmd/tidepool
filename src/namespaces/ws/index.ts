// WebSocket namespace surface. v0.1 only polyfills signatureSubscribe; any
// future WS polyfills land here as sibling files and get re-exported.

export {
  startSignatureSubscribe,
  stopSignatureSubscribe,
} from "./signature-subscribe.js";
export type { SigSubscription, SubscribeInput } from "./signature-subscribe.js";
