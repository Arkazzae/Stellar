import { createServer } from "node:http";
import { createHash, randomBytes, createHmac } from "node:crypto";
import { readFile, realpath } from "node:fs/promises";
import { createRoom, fail } from "./room.js";
import { isSdkPath, localAsset, localManifest, roomOptions, serveAsset } from "./files.js";
import { json, problem, readJson } from "./http.js";
import { createTileRelay } from "./tiles.js";

const parentFiles = new Map([
  ["/", "local.html"], ["/_stellarr/host/local.js", "local.js"],
  ["/_stellarr/host/media-host.js", "media-host.js"], ["/_stellarr/host/devices.js", "devices.js"],
]);
const maxPersonalRooms = 128;

export async function startLocal(root, port = 4174, { mode, clock = Date.now } = {}) {
  root = await realpath(root);
  const manifest = await localManifest(root);
  if (mode !== undefined && !["voice", "personal"].includes(mode))
    throw new Error("Choose --mode personal or --mode voice.");
  const modes = manifest.launchModes ?? ["voice"];
  const launchMode = mode ?? (modes.includes("voice") ? "voice" : "personal");
  if (!modes.includes(launchMode)) throw new Error(`This activity does not declare the ${launchMode} launch mode.`);
  const options = { ...roomOptions(manifest), clock };
  const createScope = () => ({ room: createRoom(options), tiles: createTileRelay(clock) });
  const shared = launchMode === "voice" ? createScope() : undefined;
  const personal = new Map();

  function activePersonalRoom(id) {
    const scope = personal.get(id);
    if (!scope?.room.expired()) return scope;
    personal.delete(id);
  }

  function prunePersonalRooms() {
    for (const id of personal.keys()) activePersonalRoom(id);
  }

  const secret = randomBytes(32);
  const hostname = "game-" + createHash("sha256").update(root).digest("hex").slice(0,12) + ".localhost";
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error("Invalid port.");
  const server = createServer(async (req, res) => {
    try {
      const listenPort = server.address().port;
      const origin = "http://127.0.0.1:" + listenPort;
      const frame = "http://" + hostname + ":" + listenPort;
      const host = "http://" + req.headers.host;
      const path = decodeURIComponent(new URL(req.url, origin).pathname);
      const read = req.method === "GET" || req.method === "HEAD";
      if (host === frame) {
        if (!read) fail(405, "Expected GET.");
        const name = isSdkPath(path) ? path : path.slice(1) || "index.html";
        return await serveAsset(res, name, isSdkPath(name) ? null : await localAsset(root, name), manifest, origin, req);
      }
      if (host !== origin) fail(403, "Unexpected host.");
      if (read && parentFiles.has(path)) {
        const bytes = await readFile(new URL("../host/" + parentFiles.get(path), import.meta.url));
        res.writeHead(200, { "Content-Type": path === "/" ? "text/html" : "text/javascript",
          "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src blob:; frame-src " + frame + "; frame-ancestors 'none'; object-src 'none'",
          "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" });
        return res.end(bytes);
      }
      if (read && path === "/_stellarr/config") return json(res, { name: manifest.name, permissions: manifest.permissions ?? [], frame });
      if (!["/_stellarr/dev/room", "/_stellarr/dev/tile"].includes(path)) fail(404, "File not found.");
      if (req.method !== "POST" || req.headers.origin !== origin) fail(403, "Unexpected origin.");
      const { token, action } = await readJson(req);
      if (typeof token !== "string" || !/^[a-f0-9-]{36}$/.test(token)) fail(400, "Invalid player token.");
      const id = createHmac("sha256", secret).update(token).digest("hex").slice(0,24);
      const tile = path.endsWith("/tile");
      const opening = !tile && action?.op === "open";
      if (tile && !manifest.permissions?.includes("media.cameraTile")) fail(403, "Camera not declared.");
      const existing = shared ?? activePersonalRoom(id);
      if (!existing) {
        if (!opening) fail(404, "Open the personal preview first.");
        if (personal.size >= maxPersonalRooms) prunePersonalRooms();
        if (personal.size >= maxPersonalRooms) fail(429, "The local simulator has too many active personal rooms.");
      }
      const scope = existing ?? createScope();
      if (tile) return json(res, scope.tiles(id, action));
      const response = scope.room({ id, name: "Player " + id.slice(0,4) }, action);
      if (!existing) personal.set(id, scope);
      return json(res, opening ? { ...response, launchMode } : response);
    } catch (error) { problem(res, error); }
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
  if (!shared) {
    const timer = setInterval(prunePersonalRooms, 30_000);
    timer.unref();
    server.once("close", () => clearInterval(timer));
  }
  return { server, origin: "http://127.0.0.1:" + server.address().port, manifest };
}
