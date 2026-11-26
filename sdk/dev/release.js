import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { chmod, mkdir, mkdtemp, readFile, realpath, rename, rm, stat, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { promisify } from "node:util";
import { readRelease, releaseBytesLimit } from "./files.js";

const run = promisify(execFile);
const archiveTime = new Date("2000-01-01T00:00:00Z");

async function makeZip(source, output, names) {
  if (process.platform === "win32") {
    const listing = output + ".list";
    await writeFile(listing, JSON.stringify(names));
    const command = `
      $ErrorActionPreference = 'Stop'
      Add-Type -AssemblyName System.IO.Compression
      Add-Type -AssemblyName System.IO.Compression.FileSystem
      $names = Get-Content -LiteralPath $env:STELLARR_ZIP_LIST -Encoding UTF8 -Raw | ConvertFrom-Json
      $zip = [IO.Compression.ZipFile]::Open($env:STELLARR_ZIP_DEST, [IO.Compression.ZipArchiveMode]::Create)
      try {
        foreach ($name in $names) {
          $path = [IO.Path]::Combine($env:STELLARR_ZIP_SOURCE, $name)
          $entry = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $path, $name)
          $entry.LastWriteTime = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
        }
      } finally { $zip.Dispose() }
    `;
    try {
      await run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", command], {
        env: { ...process.env, STELLARR_ZIP_SOURCE: source, STELLARR_ZIP_DEST: output, STELLARR_ZIP_LIST: listing },
      });
    } catch (error) {
      if (error.code === "ENOENT") throw new Error("Install Windows PowerShell to package an activity release.");
      throw error;
    }
    return;
  }
  try {
    const zip = run("zip", ["-X", "-q", "-D", "-nw", output, "-@"], {
      cwd: source, env: { ...process.env, TZ: "UTC", ZIPOPT: "" },
    });
    zip.child.stdin.on("error", () => {});
    zip.child.stdin.end(names.map(name => "./" + name).join("\n") + "\n");
    await zip;
  } catch (error) {
    if (error.code === "ENOENT") throw new Error("Install the zip command to package an activity release.");
    throw error;
  }
}

export async function packageRelease(directories, destination) {
  if (!directories.length || directories.length > 64) throw new Error("Select 1 to 64 published activity folders.");
  const output = resolve(destination);
  await mkdir(output, { recursive: true });
  const canonicalOutput = await realpath(output);
  const inputs = await Promise.all(directories.map(dir => realpath(dir)));
  if (inputs.some(input => input === canonicalOutput)) throw new Error("Choose a release output separate from the activity root.");
  const staging = await mkdtemp(join(canonicalOutput, ".release-"));
  let sources;
  const activities = [];
  try {
    sources = await mkdtemp(join(tmpdir(), "stellarr-release-"));
    const ids = new Set();
    for (const dir of inputs) {
      const { manifest, assets } = await readRelease(dir, canonicalOutput);
      const { id, name, version } = manifest;
      if (ids.has(id)) throw new Error("Duplicate activity ID in this release: " + id);
      ids.add(id);
      const source = join(sources, id);
      const names = [...assets.keys()].sort();
      for (const file of names) {
        const target = join(source, file);
        await mkdir(dirname(target), { recursive: true });
        await writeFile(target, assets.get(file), { flag: "wx" });
        if (process.platform !== "win32") await chmod(target, 0o644);
        await utimes(target, archiveTime, archiveTime);
      }
      const temporaryZip = join(staging, id + ".zip");
      await makeZip(source, temporaryZip, names);
      const { size } = await stat(temporaryZip);
      if (!size || size > releaseBytesLimit) throw new Error("Release ZIP exceeds the instance's 256 MiB limit.");
      const data = await readFile(temporaryZip);
      const sha256 = createHash("sha256").update(data).digest("hex");
      const asset = id + "-" + sha256.slice(0, 16) + ".zip";
      await rename(temporaryZip, join(staging, asset));
      activities.push({ id, name, version, asset, sha256, bytes: data.length });
    }
    activities.sort((first, second) => first.id < second.id ? -1 : first.id > second.id ? 1 : 0);
    const index = "stellarr-release.json";
    await writeFile(join(staging, index), JSON.stringify({ protocol: 1, activities }, null, 2) + "\n");
    for (const { asset } of activities) await rename(join(staging, asset), join(canonicalOutput, asset));
    await rename(join(staging, index), join(canonicalOutput, index));
    return activities;
  } finally {
    await Promise.all([staging, sources].filter(Boolean).map(folder => rm(folder, { recursive: true, force: true })));
  }
}
