import { randomBytes, createHash } from "node:crypto";
import { createRoom, fail, record } from "./room.js";
import { json, problem, readJson } from "./http.js";
import { isSdkPath, roomOptions, serveAsset, unpack, uploadLimit } from "./files.js";
import { previewProgress } from "./progress.js";
import { createTileRelay } from "./tiles.js";

const secret = () => randomBytes(18).toString("hex");
const digest = value => createHash("sha256").update(value).digest("hex");
const minute = 60_000;
const duration = { pairCode: 5 * minute, author: 24 * 60 * minute, draft: 2 * 60 * minute };
const prefix = "/_stellarr/dev/";
const tokenPattern = /^[a-f0-9]{36}$/;

// Mounted only by the explicitly enabled Vite development server. The dedicated
// asset hosts never fall through to Vite or its API and filesystem proxies.
export function previewMiddleware({ origin, domain, apiOrigin, clock = Date.now }) {
  const parent = new URL(origin);
  if (parent.origin !== origin || !/^[a-z0-9-]+(?:\.[a-z0-9-]+)*$/.test(domain) ||
      parent.hostname === domain || parent.hostname.endsWith("." + domain) ||
      !(parent.protocol === "https:" || (parent.protocol === "http:" &&
        ["localhost", "127.0.0.1"].includes(parent.hostname) && domain.endsWith(".localhost"))))
    throw new Error("Invalid activity preview origin or dedicated asset domain.");
  const pairCodes = new Map();
  const authors = new Map();
  const drafts = new Map();
  const hosts = new Map();

  function removeLease(draft, access, revoke) {
    const lease = draft.leases.get(access);
    if (!lease) return;
    draft.leases.delete(access);
    for (const [token, ticket] of draft.tickets) if (ticket.access === access) draft.tickets.delete(token);
    if (revoke) {
      draft.room.revoke(lease.person);
      draft.tiles(lease.person, { op: "clear" });
    }
  }

  function removeDraft(code, draft, revokeAuthor = true) {
    drafts.delete(code);
    hosts.delete(draft.host);
    draft.room.end();
    draft.author.activeDraft = undefined;
    if (revokeAuthor) authors.delete(draft.author.id);
  }

  function expire() {
    const now = clock();
    for (const [id, until] of pairCodes) if (now >= until) pairCodes.delete(id);
    for (const [id, author] of authors) {
      if (now < author.until) continue;
      if (author.activeDraft) removeDraft(author.activeDraft, drafts.get(author.activeDraft));
      else authors.delete(id);
    }
    for (const [code, draft] of drafts) {
      if (now >= draft.expires) {
        removeDraft(code, draft);
      } else {
        for (const [access, lease] of draft.leases)
          if (now >= lease.until) removeLease(draft, access, true);
        for (const [token, ticket] of draft.tickets)
          if (now >= ticket.until) draft.tickets.delete(token);
      }
    }
  }

  const interval = setInterval(expire, minute);
  interval.unref();

  function instanceRequest(req, path, kind, version) {
    return fetch(new URL(path, apiOrigin), {
      headers: { Cookie: req.headers.cookie ?? "", "X-Stellarr-Client": kind, "X-Stellarr-Client-Version": version },
      redirect: "error", signal: AbortSignal.timeout(5000),
    }).catch(() => fail(503, "The test instance could not verify access."));
  }

  async function seated(req, channel) {
    if (typeof channel !== "string" || !/^[a-f\d-]{36}$/i.test(channel)) fail(400, "Invalid voice channel.");
    const response = await instanceRequest(req, "/api/dev/activity-seat/" + channel, "web", "2.0.0");
    if (response.status === 401 || response.status === 403)
      fail(response.status, "Join the voice channel on the test instance first.");
    if (!response.ok) fail(503, "The test instance could not confirm the voice seat.");
    const person = await response.json();
    if (!record(person) || typeof person.id !== "string" || typeof person.name !== "string")
      fail(503, "The test instance did not return a participant.");
    return person;
  }

  async function issuePairCode(req, res) {
    if (req.headers.origin !== origin) fail(403, "Unexpected origin.");
    const response = await instanceRequest(req, "/api/dev/activity-author", "cockpit", "0.1.0");
    await response.body?.cancel();
    if (response.status === 404) fail(404, "Private pairing needs a Development API.");
    if (response.status === 401 || response.status === 403)
      fail(response.status, "Sign in to Cockpit as the instance owner to connect a project.");
    if (!response.ok) fail(503, "The test instance could not verify Cockpit access.");
    if (pairCodes.size >= 32) fail(429, "Too many pending pairing codes. Wait for a code to expire.");
    const code = secret();
    const expiresAt = clock() + duration.pairCode;
    pairCodes.set(digest(code), expiresAt);
    return json(res, { code, expiresAt, instance: origin });
  }

  async function pair(req, res) {
    const input = await readJson(req);
    if (typeof input?.code !== "string" || !tokenPattern.test(input.code)) fail(403, "Invalid or expired pairing code.");
    const id = digest(input.code);
    if (!(pairCodes.get(id) > clock())) fail(403, "Invalid or expired pairing code.");
    if (authors.size >= 64) fail(429, "All author pairing slots are in use.");
    pairCodes.delete(id);
    const token = secret();
    const expiresAt = clock() + duration.author;
    const authorId = digest(token);
    authors.set(authorId, { id: authorId, until: expiresAt, progress: new Map() });
    return json(res, { token, expiresAt });
  }

  function authorOf(req) {
    const match = /^Bearer ([a-f0-9]{36})$/.exec(req.headers.authorization ?? "");
    const author = match && authors.get(digest(match[1]));
    if (!(author?.until > clock())) fail(403, "The author pairing expired. Connect the project in Cockpit again.");
    return author;
  }

  async function authorRequest(req, res, path) {
    const author = authorOf(req);
    const input = await readJson(req, path === "drafts" ? uploadLimit : undefined);
    expire();
    if (authors.get(author.id) !== author) fail(403, "The author pairing expired. Connect the project in Cockpit again.");
    if (path === "stop") {
      const draft = drafts.get(input?.code);
      if (draft?.author !== author) fail(404, "Unknown or expired preview for this paired project.");
      removeDraft(input.code, draft);
      return json(res, { stopped: true });
    }
    if (drafts.size >= 8 && !author.activeDraft) fail(429, "All eight draft slots are in use.");
    const draft = unpack(input?.files);
    if (author.activity !== undefined && author.activity !== draft.manifest.id)
      fail(403, "This pairing belongs to another activity. Connect this project in Cockpit first.");
    author.activity = draft.manifest.id;
    const code = secret();
    const frame = new URL(origin);
    frame.hostname = secret() + "." + domain;
    const expiresAt = Math.min(clock() + duration.draft, author.until);
    Object.assign(draft, { author, host: frame.host, origin: frame.origin, room: createRoom(roomOptions(draft.manifest)),
      tiles: createTileRelay(), channel: null, expires: expiresAt, tickets: new Map(), leases: new Map() });
    if (author.activeDraft) removeDraft(author.activeDraft, drafts.get(author.activeDraft), false);
    author.activeDraft = code;
    drafts.set(code, draft);
    hosts.set(frame.host, draft);
    return json(res, { code, expiresAt });
  }

  function asset(req, res, draft, url) {
    if (!draft || !["GET", "HEAD"].includes(req.method)) fail(404, "Unknown preview.");
    if (url.pathname === "/_stellarr/launch") {
      const token = url.searchParams.get("ticket");
      const ticket = draft.tickets.get(token);
      if (!ticket || !(draft.leases.get(ticket.access)?.until > clock()))
        fail(403, "Open the activity from your voice channel.");
      draft.tickets.delete(token);
      res.writeHead(302, {
        Location: "/", "Cache-Control": "no-store", "Referrer-Policy": "no-referrer",
        "Set-Cookie": "__Host-stellarr-preview=" + ticket.access + "; Path=/; Secure; HttpOnly; SameSite=None; Partitioned",
      });
      return res.end();
    }
    const cookie = req.headers.cookie?.split(/;\s*/).find(pair => pair.startsWith("__Host-stellarr-preview="))?.split("=")[1];
    if (!(draft.leases.get(cookie)?.until > clock())) fail(403, "Open the activity from your voice channel.");
    const path = decodeURIComponent(url.pathname);
    if (path.startsWith("/_stellarr/avatars/")) {
      const photo = [...draft.leases.values()].find(lease => lease.avatar?.path === path)?.avatar;
      if (!photo?.bytes) fail(404, "Unknown participant avatar.");
      res.writeHead(200, { "Content-Type": photo.type, "Cache-Control": "private, no-store", "X-Content-Type-Options": "nosniff" });
      return res.end(photo.bytes);
    }
    const sdk = isSdkPath(path);
    const name = sdk ? path : path.slice(1) || "index.html";
    const bytes = draft.assets.get(name);
    if (!sdk && !bytes) fail(404, "File not found.");
    return serveAsset(res, name, bytes, draft.manifest, origin, req);
  }

  async function testerRequest(req, res, path) {
    if (req.headers.origin !== origin) fail(403, "Unexpected origin.");
    const input = await readJson(req);
    if (!record(input)) fail(400, "Expected a preview request.");
    const draft = drafts.get(input.code);
    if (!draft || draft.expires <= clock()) fail(404, "Unknown or expired invite code.");
    if (draft.channel && draft.channel !== input.channel) fail(403, "The preview belongs to another voice channel.");
    const lease = draft.leases.get(input.access);
    if (path !== "launch" && !(lease?.until > clock())) fail(403, "Open the preview from your voice channel.");
    let person;
    try {
      person = await seated(req, input.channel);
    } catch (error) {
      if (path !== "launch" && [401, 403].includes(error.status)) removeLease(draft, input.access, true);
      throw error;
    }
    if (drafts.get(input.code) !== draft || draft.expires <= clock())
      fail(404, "Unknown or expired invite code.");
    if (draft.channel && draft.channel !== input.channel) fail(403, "The preview belongs to another voice channel.");
    if (path === "launch") {
      if (draft.manifest.devices && !draft.manifest.devices.includes(input.device ?? "desktop"))
        fail(400, "This release does not support this device.");
      if (draft.manifest.launchModes?.includes("voice") === false)
        fail(400, "This activity supports personal launches. Run stellarr dev locally to preview it.");
      for (const [access, previous] of draft.leases) if (previous.person === person.id) removeLease(draft, access, false);
      if (draft.tickets.size + draft.leases.size >= 256) fail(429, "Preview launch limit reached.");
      draft.channel = input.channel;
      const ticket = secret();
      const access = secret();
      draft.leases.set(access, { person: person.id, until: clock() + minute });
      draft.tickets.set(ticket, { until: clock() + minute / 2, access });
      return json(res, { title: draft.manifest.name, access, src: draft.origin + "/_stellarr/launch?ticket=" + ticket,
        permissions: draft.manifest.permissions ?? [] });
    }
    if (draft.leases.get(input.access) !== lease || lease.person !== person.id || lease.until <= clock())
      fail(403, "Open the preview from your voice channel.");
    lease.until = clock() + minute;
    if (path === "tile") {
      if (!draft.manifest.permissions?.includes("media.cameraTile")) fail(403, "This activity did not request camera tiles.");
      return json(res, draft.tiles(person.id, input.action));
    }
    const asset = typeof person.avatar === "string" && /^\/assets\/([a-f0-9-]{36})$/i.exec(person.avatar);
    if (!asset) {
      delete lease.avatar;
      person.avatar = null;
    } else {
      const path = "/_stellarr/avatars/" + person.id + "/" + asset[1];
      if (lease.avatar?.path !== path || (!lease.avatar.bytes && clock() >= lease.avatar.retryAt)) {
        const photo = lease.avatar = { path, retryAt: clock() + 5000 };
        const image = await instanceRequest(req, person.avatar, "web", "2.0.0").catch(() => null);
        if (image?.ok && /^image\/(png|jpeg|gif|webp)$/.test(image.headers.get("content-type")) &&
            Number(image.headers.get("content-length") ?? Infinity) <= 4 * 1024 * 1024) {
          photo.bytes = Buffer.from(await image.arrayBuffer());
          photo.type = image.headers.get("content-type");
        } else await image?.body?.cancel();
        if (draft.leases.get(input.access) !== lease || drafts.get(input.code) !== draft) fail(403, "Preview closed.");
      }
      person.avatar = lease.avatar?.bytes ? path : null;
    }
    const response = draft.room(person, input.action);
    if (input.action.op === "saveProgress" || input.action.op === "loadProgress") {
      const saves = draft.author.progress;
      return json(res, previewProgress(saves.get(person.id), input.action, next => saves.set(person.id, next)));
    }
    if (input.action.op === "open") Object.assign(response, { progress: true, embedHosts: (draft.manifest.connects ?? []).map(c => c.host) });
    if (input.action?.op === "close") removeLease(draft, input.access, true);
    return json(res, { ...response, launchMode: "voice", place: { id: draft.channel } });
  }

  async function handle(req, res, next) {
    const host = (req.headers.host ?? "").toLowerCase();
    const url = new URL(req.url, origin);
    const hostname = host.split(":")[0];
    if (hostname === domain || hostname.endsWith("." + domain)) {
      expire();
      return asset(req, res, hosts.get(host), url);
    }
    if (!url.pathname.startsWith(prefix)) return next();
    if (host !== parent.host || req.method !== "POST") fail(403, "Unexpected preview request.");
    expire();
    const path = url.pathname.slice(prefix.length);
    if (path === "pair-code") return issuePairCode(req, res);
    if (path === "pair") return pair(req, res);
    if (path === "drafts" || path === "stop") return authorRequest(req, res, path);
    if (["launch", "room", "tile"].includes(path)) return testerRequest(req, res, path);
    fail(404, "Unknown preview request.");
  }

  return {
    middleware: (req, res, next) => handle(req, res, next).catch(error => problem(res, error)),
    close() {
      clearInterval(interval);
      for (const [code, draft] of drafts) removeDraft(code, draft);
      pairCodes.clear();
      authors.clear();
    },
  };
}
