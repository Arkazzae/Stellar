import { fileURLToPath } from "node:url";

import react from "@vitejs/plugin-react";
import { configDefaults, defineConfig } from "vitest/config";

import { stellarrArtifactSizes } from "./artifactSizes";

// Suites that can run without the browser globals, locale or storage from vitest.setup.ts.
const nodeTests = [
  "bundleReport.test.ts",
  "src/stellarr/components/room/attachments/{imageBox,players}.test.ts",
  "src/stellarr/components/room/panels/{channelPanel,channelPanelModel.behaviour}.test.ts",
  "src/stellarr/components/room/transcript/{scrollAnchor,transcriptRows}.test.ts",
  "src/stellarr/components/settings/sound/WaveformTrimmer.decode.test.ts",
  "src/stellarr/components/shell/rail/voiceMove.test.ts",
  "src/stellarr/design/{identity,identityColour,themeAccent,themeFile}.test.ts",
  "src/stellarr/i18n/{catalogs,polishText}.test.ts",
  "src/stellarr/lib/{artifacts,stellarr-ui}.test.ts",
  "src/stellarr/lib/api/{*Parsers,clientExports,expect,parsers}.test.ts",
  "src/stellarr/lib/auth/{passwordStrength,qr,query,returnPath,username,webauthn}.test.ts",
  "src/stellarr/lib/{chatTranslation,e2ee,rtc,wallpaper}/*.test.ts",
  "src/stellarr/lib/media/{imagePreview,measureUpload,placeholder,placeholderUrl}.test.ts",
  "src/stellarr/lib/messages/{codeLanguages,codeLanguages.resolve,codeTokenizer,composerText,inline,messageSyntaxContract,unread}.test.ts",
  "src/stellarr/lib/realtime/{deferred,emitter,hubPayloads}.test.ts",
  "src/stellarr/lib/voice/{audioProfiles,mediaSession,participantPreferences,participantPreferences.storage,settings,settingsStore,store}.test.ts",
  "src/stellarr/lib/voice/{capture/detector,characterization,e2ee}/*.test.ts",
  "src/stellarr/lib/voice/rtc/{bitrate,iceConfig,iceSession,sdp,sdp.msid,stats}.test.ts",
  "src/stellarr/ports/index.test.ts",
  "src/stellarr/state/{directoryView,dmTimeline,voiceHolder,voiceSeats,voiceView}.test.ts",
  "src/stellarr/state/chat/{chatStore,firstPage,pendingChips,pins,publish,readMarks,threads,timeline}.test.ts",
];

// The test scripts load this file with `--configLoader runner`, as the build
// does vite.config.ts, so the local import above needs no file extension.
export default defineConfig({
  // The sizes of the prebuilt files, which the client reads as the build gives them.
  plugins: [react(), stellarrArtifactSizes()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src/stellarr", import.meta.url)),
    },
  },
  test: {
    // A hung-test guard that leaves time for userEvent on a busy runner.
    testTimeout: 20_000,
    // The web client and Cockpit are two apps with their own test setup: the
    // web client's starts every test on a Polish page with an in-memory
    // localStorage, while Cockpit picks its language itself and its tests use
    // the browser's storage as it is. `vitest --project cockpit` runs one.
    projects: [
      {
        extends: true,
        test: {
          name: "web-node",
          environment: "node",
          globals: true,
          include: nodeTests,
        },
      },
      {
        extends: true,
        test: {
          name: "web",
          environment: "jsdom",
          globals: true,
          setupFiles: ["./vitest.setup.ts"],
          // documents.test.ts checks the two documents the build writes,
          // and release.test.ts the id the build gives the release.
          include: ["src/**/*.test.{ts,tsx}", "documents.test.ts", "release.test.ts"],
          exclude: [...configDefaults.exclude, "src/cockpit/**", ...nodeTests],
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
