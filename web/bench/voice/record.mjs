// Rewrites the voice records: the capture record through Vitest, the playback
// record through Chrome. Run it only for a commit whose message names the
// audible change it intends; see README.md beside this file.
//
//   npm run voice:record -w stellarr-web
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

const webRoot = resolve(import.meta.dirname, "..", "..");

function run(command, args, env = {}) {
  console.log(`\n$ ${[command, ...args].join(" ")}`);
  const result = spawnSync(command, args, {
    cwd: webRoot,
    stdio: "inherit",
    env: { ...process.env, ...env },
    shell: process.platform === "win32",
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run(
  "npx",
  ["vitest", "run", "--configLoader", "runner", "--project", "web", "src/stellarr/lib/voice/characterization/"],
  { STELLARR_VOICE_RECORD: "1" },
);
run(process.execPath, ["bench/voice/playback/run.mjs", "--record", "--repeat", "5", ...process.argv.slice(2)]);
console.log("\nRecorded. Review the diff of web/bench/voice/records/ before committing it.");
