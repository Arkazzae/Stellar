// What the channel rail redraws, and why, in three real browsers against a
// running instance: Ala, Bob and Cyd, each with a WAV file for a microphone.
// Every page records, on its own clock:
//
//   - each React commit: how many components rendered, and which of them in
//     the rail (the subtree under RailContainer), through the DevTools hook;
//   - the occupant rows of the rail: class changes with the old value, rows
//     added and removed, and animations that start inside the rail;
//   - what the speech detector posts to the main thread, per detector node;
//   - a snapshot of the rail after each commit that changed it.
//
// The scenarios run in order and are marked on every page at once, so each
// measurement falls into a named window.
//
//   node bench/rail/updates/probe.mjs --origin http://127.0.0.1:3110 \
//     --mics <dir with mic-a.wav mic-b.wav mic-c.wav> --container stellarr-railprobe-api-1 \
//     --out result.json [--scenario full|quiet] [--label before]
import { spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const here = import.meta.dirname;
const { findChromium, launch, sleep } = await import(
  pathToFileURL(resolve(here, "..", "..", "cdp.mjs")).href
);

function parseArgs(argv) {
  const options = {
    origin: "http://127.0.0.1:3110",
    mics: null,
    container: "stellarr-railprobe-api-1",
    out: null,
    label: "run",
    // "full", or "quiet": presence and messages only, nobody in a call.
    scenario: "full",
  };
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index].replace(/^--/, "");
    if (!(key in options)) throw new Error(`Unknown option ${argv[index]}`);
    options[key] = argv[index + 1];
  }
  if (options.mics === null) throw new Error("--mics <dir> is required");
  options.origin = options.origin.replace(/\/$/, "");
  return options;
}

// ---------------------------------------------------------------- the probe

const PROBE = String.raw`(() => {
  const T = () => Date.now();
  const P = (window.__probe = {
    commits: [], dom: [], anim: [], vad: [], vadNodes: [], snaps: [], marks: [], sockets: [],
    window: "setup", all: {},
  });
  P.mark = (label) => { P.marks.push([T(), label]); P.window = label; };

  // React reports every commit to the DevTools hook, production build included.
  const PERFORMED_WORK = 1;
  const COMPONENTS = new Set([0, 1, 11, 14, 15]);
  const nameOf = (fiber) => {
    const type = fiber.type;
    if (type == null) return "?";
    return type.displayName || type.name || type.type?.displayName || type.type?.name || type.render?.name || "?";
  };
  const walk = (fiber, inRail, rec) => {
    for (let child = fiber.child; child !== null; child = child.sibling) {
      const previous = child.alternate;
      let rail = inRail;
      if (COMPONENTS.has(child.tag)) {
        const name = nameOf(child);
        if (name === "RailContainer") rail = true;
        if (previous === null || (child.flags & PERFORMED_WORK) !== 0) {
          rec.total += 1;
          rec.names[name] = (rec.names[name] ?? 0) + 1;
          if (rail) rec.rail[name] = (rec.rail[name] ?? 0) + 1;
        }
      }
      if (previous === null || child.child !== previous.child) walk(child, rail, rec);
    }
  };
  const snapshot = () => {
    const rail = document.querySelector("nav.rail");
    if (!rail) return "";
    const rows = [];
    for (const channel of rail.querySelectorAll(".channel")) {
      const name = channel.querySelector(".name")?.textContent ?? "?";
      const unread = channel.querySelector(".ch-unread")?.textContent ?? "";
      const count = channel.querySelector(".count");
      rows.push(name + (channel.classList.contains("active") ? "*" : "") + (unread ? "!" + unread : "") +
        (count && !count.hidden ? "#" + count.textContent : ""));
      const drop = channel.closest(".voice-channel-drop");
      if (drop) {
        for (const li of drop.querySelectorAll("li.voice-occupant")) {
          const who = li.querySelector(".occ-name")?.textContent ?? "?";
          const flags = [["me", "self"], ["speaking", "talk"], ["muted", "mute"], ["is-moving", "move"]]
            .filter(([c]) => li.classList.contains(c)).map(([, word]) => word).join(",");
          rows.push("  " + who + (flags ? "[" + flags + "]" : ""));
        }
      }
    }
    return rows.join("|");
  };
  let lastSnap = "";
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    isDisabled: false,
    supportsFiber: true,
    renderers: new Map(),
    inject: () => 1,
    checkDCE: () => {},
    onCommitFiberUnmount: () => {},
    onPostCommitFiberRoot: () => {},
    onCommitFiberRoot: (_id, root) => {
      const rec = { total: 0, names: {}, rail: {} };
      walk(root.current, false, rec);
      const all = (P.all[P.window] ??= {});
      for (const [name, count] of Object.entries(rec.names)) all[name] = (all[name] ?? 0) + count;
      P.commits.push([T(), P.window, rec.total, rec.rail]);
      const snap = snapshot();
      if (snap !== lastSnap) { P.snaps.push([T(), snap]); lastSnap = snap; }
    },
  };

  // The rows: class flips, adds and removes. Removed rows are detached, so
  // their channel is read from the list they left.
  const who = (li) => li.querySelector?.(".occ-name")?.textContent ?? "?";
  const channelOf = (el) => el.closest?.(".voice-channel-drop")?.querySelector(".channel .name")?.textContent ?? "?";
  const rowsIn = (node) => node instanceof Element
    ? (node.matches("li.voice-occupant") ? [node] : [...node.querySelectorAll("li.voice-occupant")])
    : [];
  const observe = () => new MutationObserver((records) => {
    const t = T();
    for (const record of records) {
      const target = record.target;
      if (record.type === "attributes") {
        if (!(target instanceof Element)) continue;
        if (target.matches("li.voice-occupant")) {
          P.dom.push([t, "cls", channelOf(target), who(target), record.oldValue, target.className]);
        } else if (target.matches("li.voice-occupant .av")) {
          P.dom.push([t, "av", channelOf(target), who(target.closest("li")), record.oldValue, target.className]);
        }
      } else if (record.type === "childList") {
        const channel = target instanceof Element ? channelOf(target) : "?";
        for (const node of record.removedNodes) for (const li of rowsIn(node)) P.dom.push([t, "remove", channel, who(li), li.className]);
        for (const node of record.addedNodes) for (const li of rowsIn(node)) P.dom.push([t, "add", channelOf(li), who(li), li.className]);
      }
    }
  }).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ["class"], attributeOldValue: true });
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", observe); else observe();
  document.addEventListener("animationstart", (event) => {
    const el = event.target;
    if (!(el instanceof Element) || !el.closest("nav.rail")) return;
    const li = el.closest("li.voice-occupant");
    P.anim.push([T(), event.animationName, li ? channelOf(li) : "", li ? who(li) : el.className]);
  }, true);

  // What the detector posts, per node.
  const NativeNode = window.AudioWorkletNode;
  if (NativeNode) {
    window.AudioWorkletNode = class extends NativeNode {
      constructor(context, name, options) {
        super(context, name, options);
        if (name !== "stellarr-vad") return;
        const options_ = options?.processorOptions ?? {};
        const id = P.vadNodes.length + 1;
        P.vadNodes.push({ id, t: T(), silentLevels: options_.publishSilentLevels !== false, gate: options_.gate === true });
        this.port.addEventListener("message", (event) => {
          const data = event.data;
          if (data && typeof data.speaking === "boolean") P.vad.push([T(), id, data.speaking ? 1 : 0, data.level]);
        });
        this.port.start();
      }
    };
  }

  // The hub's socket, so a script can drop it.
  const NativeSocket = window.WebSocket;
  window.WebSocket = new Proxy(NativeSocket, {
    construct(target, args) { const socket = new target(...args); P.sockets.push(socket); return socket; },
  });
  P.dropSockets = () => {
    let closed = 0;
    for (const socket of P.sockets) if (socket.readyState === 1) { socket.close(4000, "probe"); closed += 1; }
    return closed;
  };
})()`;

// ---------------------------------------------------------------- the people

const PASSWORD = "Rail-Probe-Check-1";
const people = {
  a: { username: "ala", display: "Ala", mic: "mic-a.wav" },
  b: { username: "bob", display: "Bob", mic: "mic-b.wav" },
  c: { username: "cyd", display: "Cyd", mic: "mic-c.wav" },
};

const api = (page, method, path, body) =>
  page.call(async (method, path, body) => {
    const headers = { Accept: "application/json", "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
    const server = sessionStorage.getItem("stellarr.active-server");
    if (server) headers["X-Stellarr-Server"] = server;
    const init = { method, headers, credentials: "same-origin" };
    if (body !== undefined) {
      headers["Content-Type"] = "application/json";
      init.body = JSON.stringify(body);
    }
    const response = await fetch(path, init);
    return { status: response.status, body: await response.text() };
  }, method, path, body);

function setupCode(container) {
  const logs = spawnSync("docker", ["logs", container], { encoding: "utf8" });
  const codes = [...`${logs.stdout ?? ""}${logs.stderr ?? ""}`.matchAll(/Setup code: ([A-Z0-9-]+)\./g)];
  const code = codes.at(-1)?.[1];
  if (code === undefined) throw new Error(`No setup code in the log of ${container}`);
  return code;
}

async function signIn(options, key, inviteCode) {
  const { page, username, display } = people[key];
  let response = await api(page, "POST", "/api/auth/login", { username, password: PASSWORD });
  if (response.status >= 400) {
    response = await api(page, "POST", "/api/auth/register", {
      username,
      displayName: display,
      email: `${username}@example.test`,
      password: PASSWORD,
      ...(inviteCode
        ? { inviteCode }
        : {
            setupCode: setupCode(options.container),
            initialServer: { name: "Probe", description: "", textChannelName: "general", voiceChannelName: "lounge" },
          }),
    });
  }
  if (response.status >= 400) throw new Error(`${key}: sign-in failed ${response.status} ${response.body}`);
}

async function openApp(options, key) {
  const { page } = people[key];
  await page.goto(`${options.origin}/`);
  await page.waitFor(`!!document.querySelector("nav.rail .channel")`, 60_000);
}

const click = (key, selector) =>
  people[key].page.call((selector) => {
    const button = document.querySelector(selector);
    if (!button) return false;
    button.click();
    return true;
  }, selector);
const joinVoice = (key, name) =>
  people[key].page.call((name) => {
    const button = [...document.querySelectorAll("nav.rail button.channel-main")]
      .find((item) => item.querySelector(".name")?.textContent.trim() === name);
    if (!button) return false;
    button.click();
    return true;
  }, name);
const hangUp = (key) => click(key, 'button[aria-label="Disconnect"]');
const toggleMute = (key) => click(key, 'button[aria-label="Microphone"]');

async function setPresence(key, label) {
  await click(key, "button.acc-open");
  await sleep(400);
  const page = people[key].page;
  const ok = await page.call((label) => {
    const button = [...document.querySelectorAll('[role="group"][aria-label="Your availability"] button')]
      .find((item) => item.textContent.includes(label));
    if (!button) return false;
    button.click();
    return true;
  }, label);
  await sleep(200);
  await page.eval(`document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);
  return ok;
}

async function markAll(label) {
  for (const key of Object.keys(people)) {
    const page = people[key].page;
    if (page) await page.call((label) => window.__probe?.mark(label), label).catch(() => undefined);
  }
  console.log(`${new Date().toISOString().slice(11, 19)} ${label}`);
}

async function hubDrop(key, seconds, during) {
  const page = people[key].page;
  await page.cmd("Network.enable");
  await page.cmd("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
  const closed = await page.eval(`window.__probe.dropSockets()`);
  const started = Date.now();
  await during();
  await sleep(Math.max(0, seconds * 1000 - (Date.now() - started)));
  await page.cmd("Network.emulateNetworkConditions", { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
  return closed;
}

async function waitConnected(key, timeout = 60_000) {
  // The reconnect pill or the dock's reconnecting headline goes away.
  await people[key].page
    .waitFor(`window.__probe.sockets.some((s) => s.readyState === 1)`, timeout)
    .catch(() => console.log(`${key}: no open socket after ${timeout / 1000} s`));
}

const docker = (...args) => spawnSync("docker", args, { encoding: "utf8" });

async function apiReady(origin, timeout = 240_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${origin}/.well-known/stellarr/client`);
      if (response.ok) return true;
    } catch {
      // Not up yet.
    }
    await sleep(1000);
  }
  return false;
}


async function fullScenario(options, random) {
  // 2. Ala joins and talks from the first moment.
  await markAll("a-join-speaking");
  await joinVoice("a", "lounge");
  await sleep(9000);

  // 3. Bob joins while Ala is mid-sentence, and talks too.
  await markAll("b-join-mid-sentence");
  await joinVoice("b", "lounge");
  await sleep(8000);

  // 4. Ala mutes and unmutes while she is still talking.
  await markAll("a-mute-while-speaking");
  await toggleMute("a");
  await sleep(4000);
  await markAll("a-unmute-while-speaking");
  await toggleMute("a");
  await sleep(6000);

  // 5. Both pause; then the call runs on its own timers.
  await markAll("pause");
  await sleep(16000);

  // 6. Cyd joins, moves and leaves.
  await markAll("c-join");
  await joinVoice("c", "lounge");
  await sleep(6000);
  await markAll("c-move");
  await joinVoice("c", "studio");
  await sleep(6000);
  await markAll("c-leave");
  await hangUp("c");
  await sleep(5000);

  // 7. Cyd's presence changes; nobody in the call cares.
  await markAll("c-presence");
  note(`idle: ${await setPresence("c", "Idle")}`);
  await sleep(3000);
  note(`online: ${await setPresence("c", "Online")}`);
  await sleep(3000);

  // 8. Messages in a channel Ala is not reading.
  await markAll("message-elsewhere");
  for (const text of ["first message elsewhere", "second message elsewhere"]) {
    const sent = await api(people.c.page, "POST", `/api/channels/${random.id}/messages`, { content: text, attachmentIds: [] });
    note(`message ${sent.status}`);
    await sleep(3000);
  }

  // 9. Cyd, outside the call, loses the hub for a while; Bob moves and mutes meanwhile.
  await markAll("c-drop");
  const cClosed = await hubDrop("c", 12, async () => {
    await sleep(1500);
    await joinVoice("b", "studio");
    await sleep(3000);
    await toggleMute("b");
    await sleep(2000);
  });
  note(`c closed ${cClosed} socket(s)`);
  await waitConnected("c");
  await markAll("c-after-drop");
  await sleep(8000);

  // 10. Ala, in the call and muted, loses the hub; Bob moves back meanwhile.
  await markAll("a-mute-before-drop");
  await toggleMute("a");
  await sleep(3000);
  await markAll("a-drop");
  const aClosed = await hubDrop("a", 12, async () => {
    await sleep(1500);
    await joinVoice("b", "lounge");
    await sleep(3000);
  });
  note(`a closed ${aClosed} socket(s)`);
  await waitConnected("a");
  await markAll("a-after-drop");
  await sleep(10000);

  // 11. The API restarts; Cyd sits in studio and closes the tab while it is down.
  await markAll("c-in-studio");
  await joinVoice("c", "studio");
  await sleep(6000);
  await markAll("restart");
  note(`stop: ${docker("stop", options.container).status}`);
  await sleep(3000);
  const cData = await people.c.page.eval(`({ __probe: window.__probe ? { commits: window.__probe.commits, dom: window.__probe.dom, anim: window.__probe.anim, vad: window.__probe.vad, vadNodes: window.__probe.vadNodes, snaps: window.__probe.snaps, marks: window.__probe.marks, all: window.__probe.all } : null })`);
  result.pages.c = cData.__probe;
  await Promise.race([people.c.browser.close(), sleep(5000)]);
  people.c.page = null;
  people.c.browser = null;
  note("c closed the browser");
  note(`start: ${docker("start", options.container).status}`);
  note(`api ready: ${await apiReady(options.origin)}`);
  await waitConnected("a", 90_000);
  await waitConnected("b", 90_000);
  await markAll("after-restart");
  await sleep(15000);
}

// ---------------------------------------------------------------- the run

const options = parseArgs(process.argv.slice(2));
const result = { label: options.label, origin: options.origin, at: new Date().toISOString(), timeline: [], pages: {} };
const note = (text) => {
  result.timeline.push([Date.now(), text]);
  console.log(`  ${text}`);
};

try {
  process.env.STELLARR_CALL_CHROME = findChromium();
  process.env.STELLARR_CHROMIUM = join(here, "chrome.sh");
  for (const key of Object.keys(people)) {
    process.env.STELLARR_CALL_MICROPHONE = resolve(options.mics, people[key].mic);
    const browser = await launch({ headless: true, width: 1280, height: 860 });
    const page = await browser.newPage();
    await page.cmd("Page.addScriptToEvaluateOnNewDocument", { source: PROBE });
    await page.goto(`${options.origin}/login`);
    Object.assign(people[key], { browser, page });
  }

  await signIn(options, "a");
  const servers = await api(people.a.page, "GET", "/api/bootstrap");
  const channels = JSON.parse(servers.body).channels ?? [];
  for (const [name, kind] of [["random", "text"], ["studio", "voice"]]) {
    if (!channels.some((channel) => channel.name === name)) {
      const created = await api(people.a.page, "POST", "/api/admin/channels", { name, kind });
      if (created.status >= 400) throw new Error(`create ${name}: ${created.status} ${created.body}`);
    }
  }
  const random = JSON.parse((await api(people.a.page, "GET", "/api/bootstrap")).body).channels.find((c) => c.name === "random");
  const invite = await api(people.a.page, "POST", "/api/admin/invites");
  const code = JSON.parse(invite.body).url?.split("/").pop();
  await signIn(options, "b", code);
  await signIn(options, "c", code);
  for (const key of Object.keys(people)) await openApp(options, key);
  await sleep(3000);

  // 1. Nobody in a call.
  await markAll("idle");
  await sleep(8000);

  if (options.scenario === "quiet") {
    await markAll("c-presence");
    note(`idle: ${await setPresence("c", "Idle")}`);
    await sleep(3000);
    note(`online: ${await setPresence("c", "Online")}`);
    await sleep(3000);
    await markAll("message-elsewhere");
    for (const text of ["first quiet message", "second quiet message"]) {
      const sent = await api(people.c.page, "POST", `/api/channels/${random.id}/messages`, { content: text, attachmentIds: [] });
      note(`message ${sent.status}`);
      await sleep(3000);
    }
  } else {
    await fullScenario(options, random);
  }
  await markAll("end");
} catch (error) {
  result.error = String(error?.stack ?? error);
  console.error(result.error);
} finally {
  for (const key of Object.keys(people)) {
    const { page } = people[key];
    if (page) {
      try {
        result.pages[key] = await page.eval(`(() => { const p = window.__probe; return { commits: p.commits, dom: p.dom, anim: p.anim, vad: p.vad, vadNodes: p.vadNodes, snaps: p.snaps, marks: p.marks, all: p.all }; })()`);
      } catch (error) {
        result.pages[key] ??= { error: String(error) };
      }
    }
  }
  if (options.out) writeFileSync(options.out, JSON.stringify(result));
  for (const { browser } of Object.values(people)) {
    try {
      await Promise.race([browser?.close(), sleep(5000)]);
    } catch {
      // Already closed.
    }
  }
}

console.log(`wrote ${options.out ?? "(nothing)"}`);
process.exit(0);
