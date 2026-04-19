import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["**/*.test.ts"],
    // The wasm-pack "web" target relies on import.meta.url + fetch to
    // pull the .wasm binary. Node 20+ supports both natively, so the
    // default node environment works — no jsdom needed.
    environment: "node",
    server: {
      deps: {
        // Vite can't compile the wasm-bindgen JS glue through its
        // bundler cache; ask Vitest to run it through its own transform
        // pipeline so import.meta.url resolves at test time.
        inline: [/solana_ratchet_wasm/],
      },
    },
  },
});
