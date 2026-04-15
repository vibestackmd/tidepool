// Debounced upstream error logging. Surfpool flapping would otherwise spam
// the console; we throttle to at most one "Surfpool not responding" block
// every 10 seconds and make it visually loud so it's hard to miss.

let lastUpstreamError = 0;

export function logUpstreamError(detail: string): void {
  const now = Date.now();
  if (now - lastUpstreamError < 10_000) return;
  lastUpstreamError = now;
  const RED = "\x1b[31m";
  const YELLOW = "\x1b[33m";
  const DIM = "\x1b[2m";
  const BOLD = "\x1b[1m";
  const R = "\x1b[0m";
  console.error(`
${RED}${BOLD}  ════════════════════════════════════════════════════${R}
${RED}${BOLD}  SURFPOOL NOT RESPONDING${R}
${RED}  ${detail}${R}

${YELLOW}  Surfpool may have crashed or stalled.${R}
${YELLOW}  Is it running?  docker compose up -d${R}
${DIM}  ════════════════════════════════════════════════════${R}
`);
}
