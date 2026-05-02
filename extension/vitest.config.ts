/// <reference types="vitest" />
import { defineConfig } from "vitest/config";
import { resolve } from "path";

// Vitest is unit-/integration-test only. Playwright owns the browser
// e2e specs under `tests/e2e/`; they import from `@playwright/test` and
// fail to load under vitest. Scope the collection to `src/` only and
// keep the same `@/` alias the rest of the build uses.
export default defineConfig({
  test: {
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    exclude: ["tests/e2e/**", "node_modules/**", "dist/**"],
  },
  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
    },
  },
});
