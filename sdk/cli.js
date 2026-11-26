#!/usr/bin/env node
import { realpath } from "node:fs/promises";
import { parseArgs } from "node:util";
import { readRelease } from "./dev/files.js";
import { warnAboutHtmlPaths } from "./dev/html-paths.js";
import { packageRelease } from "./dev/release.js";
import { startLocal } from "./dev/local.js";

async function main() {
  const { positionals, values } = parseArgs({ allowPositionals: true, options: {
    dir: { type: "string", multiple: true }, out: { type: "string" }, port: { type: "string" }, mode: { type: "string" },
    instance: { type: "string" }, stop: { type: "string" }, pair: { type: "boolean" }, help: { type: "boolean" },
  } });
  const command = positionals.join(" ");
  if (values.help || !["dev", "validate", "pack"].includes(command)) {
    console.log("stellarr dev [--dir <published folder>] [--port <port>] [--mode personal|voice]\nstellarr dev [--dir <published folder>] --instance <origin> [--pair | --stop <invite>]\nstellarr validate [--dir <published folder>]\nstellarr pack [--dir <published folder> ...] [--out <release folder>]");
    if (!values.help) process.exitCode = 1;
    return;
  }
  const directories = values.dir ?? ["."];
  const instance = values.instance !== undefined;
  const paired = instance || values.stop !== undefined || values.pair;
  if (command !== "dev" && (paired || values.port !== undefined || values.mode !== undefined))
    throw new Error("--instance, --pair, --stop, --port and --mode are options for stellarr dev.");
  if (command !== "pack" && values.out !== undefined) throw new Error("--out is an option for stellarr pack.");
  if (command === "pack") {
    const output = values.out ?? ".stellarr-release";
    if (!output.trim()) throw new Error("Choose a nonempty release output folder.");
    const activities = await packageRelease(directories, output);
    console.log("Release in " + output + ": " + activities.map(item => item.asset).join(", ") + " and stellarr-release.json");
    return;
  }
  if (directories.length !== 1) throw new Error(command + " accepts one activity folder.");
  const root = await realpath(directories[0]);
  if (command === "validate") {
    const { manifest, assets } = await readRelease(root);
    warnAboutHtmlPaths(assets);
    console.log(manifest.id + " " + manifest.version + " · " + assets.size + " files");
    return;
  }
  if (instance) {
    if (values.port !== undefined || values.mode !== undefined) throw new Error("--port and --mode are only available in a local preview.");
    if (values.stop === "") throw new Error("Enter the tester invitation to stop.");
    const { registerPreview } = await import("./dev/pairing-cli.js");
    await registerPreview({ instance: values.instance, dir: root, stop: values.stop, pair: values.pair });
    return;
  }
  if (paired) throw new Error("--stop and --pair require --instance.");
  const port = values.port ?? "4174";
  if (!/^\d+$/.test(port)) throw new Error("Invalid port.");
  const { origin, manifest } = await startLocal(root, Number(port), { mode: values.mode });
  console.log(manifest.name + " — " + origin + "/");
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
