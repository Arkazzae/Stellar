import { fileURLToPath } from "node:url";

import react from "@vitejs/plugin-react";
import { configDefaults, defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src/stellarr", import.meta.url)),
    },
  },
  test: {
    // A guard against a hung test, not a performance budget. The default 5 s
    // is generous on an idle machine, but the deploy runs this suite right
    // after a containerised `dotnet test`, and under that load the typing
    // simulated by `userEvent` tests ran past it in unrelated files at once.
    testTimeout: 20_000,
    // The web client and Cockpit are two apps with their own test setup: the
    // web client's starts every test on a Polish page with an in-memory
    // localStorage, while Cockpit picks its language itself and its tests use
    // the browser's storage as it is. `vitest --project cockpit` runs one.
    projects: [
      {
        extends: true,
        test: {
          name: "web",
          environment: "jsdom",
          globals: true,
          setupFiles: ["./vitest.setup.ts"],
          // documents.test.ts checks the two documents the build writes,
          // release.test.ts the id the build gives the release, and
          // bundleReport.test.ts the report and budget of the first document.
          include: ["src/**/*.test.{ts,tsx}", "documents.test.ts", "release.test.ts", "bundleReport.test.ts"],
          exclude: [...configDefaults.exclude, "src/cockpit/**"],
        },
      },
      {
        extends: true,
        test: {
          name: "cockpit",
          environment: "jsdom",
          setupFiles: ["./src/cockpit/test/setup.ts"],
          include: ["src/cockpit/**/*.test.{ts,tsx}"],
        },
      },
    ],
  },
});
