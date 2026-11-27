import { lstat, readdir, readFile, realpath } from "node:fs/promises";
import { extname, relative, resolve, sep } from "node:path";
import { fail, record } from "./room.js";

export const sdkPath = "/_stellarr/sdk/v1.js";
const modules = new Map([
  [sdkPath, new URL("../client.js", import.meta.url)],
  ["/_stellarr/sdk/media.js", new URL("../media.js", import.meta.url)],
]);
export const isSdkPath = path => modules.has(path);
export const roomOptions = ({ authority, seats, permissions }) => ({ authority, seats, permissions });
const types = {
  html: "text/html", css: "text/css", js: "text/javascript", mjs: "text/javascript",
  json: "application/json", txt: "text/plain", wasm: "application/wasm", svg: "image/svg+xml", png: "image/png",
  jpg: "image/jpeg", jpeg: "image/jpeg", webp: "image/webp", gif: "image/gif", ico: "image/x-icon",
  avif: "image/avif", woff: "font/woff", woff2: "font/woff2", mp3: "audio/mpeg",
  ttf: "font/ttf", otf: "font/otf", gltf: "model/gltf+json", bin: "application/octet-stream",
  data: "application/octet-stream", opus: "audio/ogg", ogg: "audio/ogg", wav: "audio/wav", mp4: "video/mp4", webm: "video/webm", glb: "model/gltf-binary",
};
const maxBytes = 16 * 1024 * 1024;
export const releaseBytesLimit = 256 * 1024 * 1024;
export const uploadLimit = Math.ceil(maxBytes * 1.4) + 262144;
const artworkLimits = { icon: 512 * 1024, cover: 2 * 1024 * 1024 };
const pngMagic = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const jpegMagic = Buffer.from([0xff, 0xd8, 0xff]);
const riffMagic = Buffer.from("RIFF");
const webpMagic = Buffer.from("WEBP");
const utf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
const maximumVersionNumber = "18446744073709551615";
const versionPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const optionalFields = ["icon", "cover", "devices", "seats", "authority"];
const manifestFields = new Set(["protocol", "id", "name", "description", "version", "permissions", "connects",
  "launchModes", ...optionalFields]);
const connectFields = new Set(["host", "why"]);

function portablePath(path) {
  return typeof path === "string" && path && path.isWellFormed() && Buffer.byteLength(path) <= 512 &&
    !/[\\:\p{Cc}]/u.test(path) && path.split("/").every(part => part && !part.startsWith(".") &&
      !["node_modules", "_stellarr", "package.json", "package-lock.json"].includes(part));
}

export function assetType(path) {
  if (!portablePath(path)) return;
  if (path === "LICENSE") return "text/plain";
  const extension = extname(path).slice(1).toLowerCase();
  return Object.hasOwn(types, extension) ? types[extension] : undefined;
}

function artworkType(path, kind) {
  if (!portablePath(path) || /[?#%]/.test(path)) fail(400, `The ${kind} must be a relative file in the activity.`);
  const type = assetType(path);
  if (!["image/png", "image/jpeg", "image/webp"].includes(type))
    fail(400, `Use a PNG, JPEG or WebP file for the ${kind}.`);
  return type;
}

function artworkError(kind) {
  fail(400, `The ${kind} must match its image extension and fit within ${artworkLimits[kind] / 1024} KiB.`);
}

function checkArtwork(manifest, assets, published = false) {
  if (published && manifest.icon === undefined)
    fail(400, "Add an icon path to stellarr.json and include the PNG, JPEG or WebP file in the activity.");
  for (const kind of Object.keys(artworkLimits)) {
    const path = manifest[kind];
    if (path === undefined) continue;
    const bytes = assets.get(path);
    if (!bytes) fail(400, `The declared ${kind} is missing from the activity: ${path}`);
    const type = artworkType(path, kind);
    const matches = type === "image/png" ? bytes.subarray(0, 8).equals(pngMagic) :
      type === "image/jpeg" ? bytes.subarray(0, 3).equals(jpegMagic) :
        bytes.subarray(0, 4).equals(riffMagic) && bytes.subarray(8, 12).equals(webpMagic);
    if (!matches || bytes.length > artworkLimits[kind]) artworkError(kind);
  }
}

function nonblankText(text, maximum) {
  return typeof text === "string" && text.isWellFormed() && Buffer.byteLength(text) <= maximum && /\P{White_Space}/u.test(text);
}

function hostname(host) {
  return typeof host === "string" && host.length <= 253 && host.includes(".") &&
    host.split(".").every(label => label.length >= 1 && label.length <= 63 &&
      !label.startsWith("-") && !label.endsWith("-") && !/[^a-z0-9-]/.test(label));
}

function semver(version) {
  const match = typeof version === "string" && versionPattern.exec(version);
  return match && match[0] === version && !match[4]?.split(".").some(part => /^0\d+$/.test(part)) &&
    match.slice(1, 4).every(part => part.length < 20 || (part.length === 20 && part <= maximumVersionNumber));
}

function afterWhitespace(source, index) {
  while (index < source.length && " \t\r\n".includes(source[index])) index++;
  return index;
}

// JSON.parse checks the grammar first. Walk the original field names because
// its result keeps only the last value, while Serde rejects duplicate known fields.
function checkDuplicateFields(source) {
  const scopes = [];
  const manifest = new Set();
  const connects = Symbol();
  let connectsAt = -1;
  for (let index = 0; index < source.length; index++) {
    switch (source[index]) {
      case "{":
        scopes.push(scopes.length === 0 ? manifest : scopes.at(-1) === connects ? new Set() : null);
        break;
      case "[":
        scopes.push(index === connectsAt ? connects : null);
        break;
      case "}":
      case "]":
        scopes.pop();
        break;
      case '"': {
        const start = index++;
        while (source[index] !== '"') index += source[index] === "\\" ? 2 : 1;
        const scope = scopes.at(-1);
        if (!(scope instanceof Set)) break;
        const colon = afterWhitespace(source, index + 1);
        if (source[colon] !== ":") break;
        const key = JSON.parse(source.slice(start, index + 1));
        const known = scope === manifest ? manifestFields : connectFields;
        if (!known.has(key)) break;
        if (scope.has(key)) fail(400, `Duplicate manifest field: ${scope === manifest ? key : "connects." + key}.`);
        scope.add(key);
        if (scope === manifest && key === "connects") connectsAt = afterWhitespace(source, colon + 1);
        break;
      }
    }
  }
}

export function manifestOf(bytes) {
  let source;
  let value;
  const nonInteger = { protocol: new WeakSet(), seats: new WeakSet() };
  try {
    source = typeof bytes === "string" ? bytes : utf8.decode(bytes);
    value = JSON.parse(source, function (key, field, context) {
      // Rust's unsigned manifest fields accept integer JSON tokens only.
      if ((key === "protocol" || key === "seats") && typeof field === "number" && !/^\d+$/.test(context.source))
        nonInteger[key].add(this);
      return field;
    });
  } catch { fail(400, "Invalid stellarr.json."); }
  if (record(value)) checkDuplicateFields(source);
  if (!record(value) || value.protocol !== 1 || nonInteger.protocol.has(value))
    fail(400, "Unsupported protocol. This SDK supports protocol 1.");
  if (typeof value.id !== "string" || !value.id || value.id.length > 100 || /[^a-z0-9-]/.test(value.id) ||
      !nonblankText(value.name, 240) || !nonblankText(value.description, 4000))
    fail(400, "Expected an activity ID, a name of up to 240 bytes and a description of up to 4000 bytes.");
  if (!semver(value.version)) fail(400, "Expected a SemVer release version.");
  // Match Rust's Option fields. Explicit null is equivalent to omission for these fields.
  for (const key of optionalFields) if (value[key] === null) delete value[key];
  if (value.authority !== undefined && !["host", "shared"].includes(value.authority)) fail(400, "Invalid authority.");
  if (value.seats !== undefined && (!Number.isInteger(value.seats) || nonInteger.seats.has(value) || value.seats < 1 || value.seats > 128))
    fail(400, "Expected 1 to 128 seats.");
  const permissions = value.permissions === undefined ? [] : value.permissions;
  if (!Array.isArray(permissions) || permissions.some(permission => typeof permission !== "string"))
    fail(400, "Expected a list of permission names.");
  for (const permission of permissions)
    if (!["media.microphoneLevel", "media.cameraTile", "media.gifs"].includes(permission)) fail(400, "Unsupported permission: " + permission);
  if (new Set(permissions).size !== permissions.length) fail(400, "Permission names must be unique.");
  if (value.devices !== undefined && (!Array.isArray(value.devices) || !value.devices.length ||
      value.devices.some(device => !["desktop", "tablet", "phone"].includes(device)))) fail(400, "Invalid supported devices.");
  if (value.launchModes !== undefined && (!Array.isArray(value.launchModes) || !value.launchModes.length ||
      new Set(value.launchModes).size !== value.launchModes.length ||
      value.launchModes.some(mode => !["voice", "personal"].includes(mode))))
    fail(400, "Expected launchModes to list voice, personal, or both without duplicates.");
  for (const kind of Object.keys(artworkLimits)) if (value[kind] !== undefined) artworkType(value[kind], kind);
  const connects = value.connects === undefined ? [] : value.connects;
  if (!Array.isArray(connects) || connects.length > 64 || connects.some(entry =>
    !record(entry) || !hostname(entry.host) || !nonblankText(entry.why, 1024)))
    fail(400, "Expected up to 64 external hosts with lowercase DNS names and reasons of up to 1024 bytes.");
  return value;
}

export async function serveAsset(res, path, bytes, manifest, parent, request) {
  if (isSdkPath(path)) bytes = await readFile(modules.get(path));
  const external = (manifest.connects ?? []).map(entry => "https://" + entry.host).join(" ");
  const headers = {
    "Content-Type": isSdkPath(path) ? "text/javascript" : assetType(path),
    "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", "Referrer-Policy": "strict-origin-when-cross-origin",
    "Content-Security-Policy": [
      "default-src 'self' " + external, "script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval' " + external,
      "style-src 'self' 'unsafe-inline' " + external, "img-src 'self' data: blob: " + external, "object-src 'none'", "base-uri 'self'",
      "form-action 'self'", "frame-ancestors " + (parent || "'none'"),
    ].join("; "),
    "Permissions-Policy": "microphone=(), camera=(), display-capture=()",
  };
  let status = 200;
  if (/^(video|audio)\//.test(headers["Content-Type"])) {
    headers["Accept-Ranges"] = "bytes";
    const range = request?.headers.range;
    if (range) {
      const match = /^bytes=(\d*)-(\d*)$/.exec(range);
      const start = match?.[1] ? Number(match[1]) : Math.max(0, bytes.length - Number(match?.[2]));
      const end = match?.[1] && match[2] ? Math.min(bytes.length - 1, Number(match[2])) : bytes.length - 1;
      if (!match || (!match[1] && !match[2]) || start > end || start >= bytes.length) {
        res.writeHead(416, { ...headers, "Content-Range": "bytes */" + bytes.length });
        return res.end();
      }
      status = 206;
      headers["Content-Range"] = "bytes " + start + "-" + end + "/" + bytes.length;
      bytes = bytes.subarray(start, end + 1);
    }
  }
  res.writeHead(status, { ...headers, "Content-Length": bytes.length });
  res.end(request?.method === "HEAD" ? undefined : bytes);
}

export async function localAsset(root, path) {
  if (!assetType(path)) fail(404, "File not found.");
  const file = resolve(root, path);
  const fromRoot = relative(root, file);
  if (!fromRoot || fromRoot === ".." || fromRoot.startsWith(".." + sep) ||
      await realpath(file).catch(() => null) !== file ||
      !(await lstat(file)).isFile()) fail(404, "File not found.");
  return readFile(file);
}

export async function localManifest(root) {
  const manifest = manifestOf(await localAsset(root, "stellarr.json"));
  const assets = new Map();
  await localAsset(root, "index.html");
  for (const kind of Object.keys(artworkLimits)) {
    const path = manifest[kind];
    if (path === undefined) continue;
    const size = await lstat(resolve(root, path)).catch(error => {
      if (error.code !== "ENOENT" && error.code !== "ENOTDIR") throw error;
      fail(400, `The declared ${kind} is missing from the activity: ${path}`);
    });
    if (size.size > artworkLimits[kind]) artworkError(kind);
    try {
      assets.set(path, await localAsset(root, path));
    } catch (error) {
      if (error.status !== 404) throw error;
      fail(400, `The declared ${kind} must be a file inside the activity: ${path}`);
    }
  }
  checkArtwork(manifest, assets);
  return manifest;
}

function finishAssets(assets, published) {
  if (!assets.has("index.html") || !assets.has("stellarr.json")) fail(400, "Missing index.html or stellarr.json.");
  const manifest = manifestOf(assets.get("stellarr.json"));
  checkArtwork(manifest, assets, published);
  return { manifest, assets };
}

async function collectAssets(root, published, omitDirectory) {
  root = await realpath(root);
  const assets = new Map();
  const filesLimit = published ? 8192 : 1024;
  const bytesLimit = published ? releaseBytesLimit : maxBytes;
  const exceedsLimit = () => fail(413, published ? "Release exceeds 256 MiB or 8192 files." : "Preview exceeds 16 MiB or 1024 files.");
  let total = 0;
  async function visit(prefix) {
    for (const entry of await readdir(resolve(root, prefix), { withFileTypes: true })) {
      const path = prefix + entry.name;
      if (!portablePath(path) || (published && entry.name === "stellarr-release.json")) continue;
      if (entry.isDirectory()) {
        if (resolve(root, path) !== omitDirectory) await visit(path + "/");
      } else if (entry.isFile() && assetType(path)) {
        if (assets.size >= filesLimit || total + (await lstat(resolve(root, path))).size > bytesLimit) exceedsLimit();
        const bytes = await localAsset(root, path);
        total += bytes.length;
        if (total > bytesLimit) exceedsLimit();
        assets.set(path, bytes);
      }
    }
  }
  await visit("");
  return finishAssets(assets, published);
}

export const readRelease = (root, omitDirectory) => collectAssets(root, true, omitDirectory);

// Private previews carry the same files as JSON instead of ZIP.
export async function pack(root) {
  const { assets } = await collectAssets(root, false);
  return Object.fromEntries([...assets].map(([path, data]) => [path, data.toString("base64")]));
}

export function unpack(files) {
  if (!record(files)) fail(400, "Missing index.html or stellarr.json.");
  const assets = new Map();
  let total = 0;
  for (const [path, value] of Object.entries(files)) {
    if (!assetType(path) || typeof value !== "string") fail(400, "Invalid activity file.");
    const bytes = Buffer.from(value, "base64");
    total += bytes.length;
    if (total > maxBytes || assets.size >= 1024) fail(413, "Preview exceeds 16 MiB or 1024 files.");
    assets.set(path, bytes);
  }
  return finishAssets(assets, false);
}
