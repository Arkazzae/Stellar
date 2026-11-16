import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// The panel is built by the web client's Next.js server (web/src/app/cockpit/);
// this configuration only runs its tests.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
  },
});
