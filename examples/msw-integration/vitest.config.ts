import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Ensure Node's native-addon loader has the quirks it needs.
    environment: "node",
    testTimeout: 15_000,
  },
});
