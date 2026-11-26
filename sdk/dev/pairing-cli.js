import { createHash, randomUUID } from "node:crypto";
import { lstat, mkdir, readFile, rename, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline/promises";
import { pack } from "./files.js";

const tokenPattern = /^[a-f0-9]{36}$/;

function instanceOrigin(input) {
  try {
    const url = new URL(input);
    const local = url.protocol === "http:" && ["localhost", "127.0.0.1"].includes(url.hostname);
    if ((url.protocol === "https:" || local) && url.pathname === "/" &&
        !url.username && !url.password && !url.search && !url.hash) return url.origin;
  } catch {
    // Report the supported form below.
  }
  throw new Error("Use the test instance origin over HTTPS, or HTTP on localhost.");
}

async function privateInfo(path) {
  const stat = await lstat(path).catch(error => {
    if (error.code !== "ENOENT") throw error;
    return null;
  });
  if (stat && (stat.isSymbolicLink() || (process.getuid && (stat.uid !== process.getuid() || (stat.mode & 0o077)))))
    throw new Error("Unsafe permissions on the Stellarr pairing store: " + path);
  return stat;
}

async function pairingFile(origin, dir) {
  const folder = join(homedir(), ".stellarr-preview");
  await mkdir(folder, { mode: 0o700 }).catch(error => {
    if (error.code !== "EEXIST") throw error;
  });
  if (!(await privateInfo(folder))?.isDirectory()) throw new Error("Expected a private Stellarr pairing directory.");
  const key = createHash("sha256").update(origin + "\n" + dir).digest("hex");
  return join(folder, key + ".json");
}

async function savedPairing(path) {
  const info = await privateInfo(path);
  if (!info) return null;
  if (!info.isFile() || info.size > 1024) throw new Error("Invalid Stellarr pairing file: " + path);
  try {
    const value = JSON.parse(await readFile(path, "utf8"));
    if (typeof value.token === "string" && tokenPattern.test(value.token) && value.expiresAt > Date.now()) return value;
  } catch {
    // A fresh pairing replaces an unreadable record.
  }
  return null;
}

async function savePairing(path, value) {
  const temporary = path + "." + randomUUID();
  await writeFile(temporary, JSON.stringify(value), { mode: 0o600, flag: "wx" });
  try {
    await rename(temporary, path);
  } finally {
    await unlink(temporary).catch(() => {});
  }
}

async function request(origin, path, body, token) {
  const response = await fetch(new URL("/_stellarr/dev/" + path, origin), {
    method: "POST", redirect: "error",
    headers: { "Content-Type": "application/json", ...(token ? { Authorization: "Bearer " + token } : {}) },
    body: JSON.stringify(body), signal: AbortSignal.timeout(20_000),
  });
  if (!response.ok) {
    const explain = (await response.text()).slice(0, 220).trim();
    throw Object.assign(new Error(explain || "The test instance refused the request (" + response.status + ")."),
      { status: response.status });
  }
  return response.json();
}

async function promptCode(origin, allowPipe) {
  if (!process.stdin.isTTY && !allowPipe)
    throw new Error("Run this command in a terminal to enter a pairing code, or pipe one line into --pair.");
  const prompt = createInterface({
    input: process.stdin,
    output: process.stdin.isTTY ? process.stdout : undefined,
    terminal: Boolean(process.stdin.isTTY),
  });
  try {
    return (await prompt.question("Pairing code for " + origin + ": ")).trim();
  } finally {
    prompt.close();
  }
}

export async function registerPreview({ instance, dir, stop, pair }) {
  const origin = instanceOrigin(instance);
  if (pair && stop) throw new Error("Use --stop with the original author pairing, without --pair.");
  const path = await pairingFile(origin, dir);
  let author = pair ? null : await savedPairing(path);
  const files = stop ? null : await pack(dir);
  if (!author) {
    if (stop) {
      await unlink(path).catch(error => { if (error.code !== "ENOENT") throw error; });
      throw new Error("The author pairing for this preview has expired.");
    }
    const code = await promptCode(origin, pair);
    if (!tokenPattern.test(code)) throw new Error("Enter the pairing code shown in Cockpit.");
    const answer = await request(origin, "pair", { code });
    if (typeof answer.token !== "string" || !tokenPattern.test(answer.token) ||
        !Number.isFinite(answer.expiresAt) || answer.expiresAt <= Date.now())
      throw new Error("The test instance returned an invalid author pairing.");
    author = { token: answer.token, expiresAt: answer.expiresAt };
    await savePairing(path, author);
  }
  try {
    if (stop) {
      await request(origin, "stop", { code: stop }, author.token);
      if ((await savedPairing(path))?.token === author.token) await unlink(path);
      console.log("Preview stopped.");
    } else {
      const { code } = await request(origin, "drafts", { files }, author.token);
      console.log("Open a voice channel on " + origin + " and choose Activity preview.\nTester invite: " + code);
    }
  } catch (error) {
    if (error.status === 403 && error.message.startsWith("The author pairing expired") &&
        (await savedPairing(path))?.token === author.token) await unlink(path);
    throw error;
  }
}
