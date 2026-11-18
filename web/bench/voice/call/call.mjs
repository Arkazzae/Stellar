// Two people in a voice call on the default settings, each with a WAV file as
// the microphone, against a running instance; what each one's browser decodes
// from the other, and whether the speaking ring it draws follows that sound.
//
//   node bench/voice/call/call.mjs --origin https://localhost:18743 [--out result.json]
//                                  [--owner a.wav --guest b.wav] [--container name]
//
// Without --owner and --guest the microphones play speech from the harness's
// fixtures, each followed by two seconds of a quiet room so that the ring has
// pauses to go out in. Chrome loops the file for as long as the call lasts.
//
// It passes when both receivers decode sound (a peak above 0.05: silence at
// the receiver is a failure whatever the cause), and when at each receiver the
// other person's ring is on for most of the time sound arrives, goes out in
// the pauses, comes on again for the next phrase, and is never on after a
// second of silence. start-container.sh starts an instance from an image with
// a build of web/ mounted. Its first account founds the instance with the
// setup code the API writes to its log at every start: the script reads it
// from `docker logs` of the container (stellarr-voice-call, the name
// start-container.sh gives, unless --container names another), or takes it
// from STELLARR_SETUP_CODE.
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { runnerImport } from "vite";

const here = import.meta.dirname;
const voiceRoot = resolve(here, "..");
const { findChromium, launch, sleep } = await import(pathToFileURL(resolve(voiceRoot, "..", "cdp.mjs")).href);

const PASS_PEAK = 0.05;
/** Received sound counts as sound above -45 dBFS RMS over 50 ms. */
const SOUND_RMS = 10 ** (-45 / 20);
const SAMPLE_MS = 50;
const WATCH_MS = 16_000;

function parseArgs(argv) {
  const options = {
    origin: "https://localhost:18743",
    out: null,
    owner: null,
    guest: null,
    container: "stellarr-voice-call",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!["--origin", "--out", "--owner", "--guest", "--container"].includes(flag) || value === undefined) {
      throw new Error(`Unknown or incomplete option ${flag}`);
    }
    options[flag.slice(2)] =
      flag === "--origin" ? value.replace(/\/$/, "") : flag === "--container" ? value : resolve(value);
    index += 1;
  }
  return options;
}

/**
 * The code the API logged at the container's last start, which the first
 * account founds the instance with. Every start draws a new one, so the last
 * line counts.
 */
function setupCode(container) {
  if (process.env.STELLARR_SETUP_CODE) return process.env.STELLARR_SETUP_CODE;
  const logs = spawnSync("docker", ["logs", container], { encoding: "utf8" });
  const codes = [...`${logs.stdout ?? ""}${logs.stderr ?? ""}`.matchAll(/Setup code: ([A-Z0-9-]+)\./g)];
  const code = codes.at(-1)?.[1];
  if (code === undefined) {
    throw new Error(`No setup code in the log of ${container}; pass --container, or set STELLARR_SETUP_CODE`);
  }
  return code;
}

/** Speech from the fixtures, then two seconds of a quiet room, as 16-bit WAV. */
async function microphoneFiles(directory) {
  const config = { configFile: false, logLevel: "error" };
  const { module: fixtures } = await runnerImport(resolve(voiceRoot, "fixtures.ts"), config);
  const { module: wav } = await runnerImport(resolve(voiceRoot, "wav.ts"), config);
  const files = {};
  for (const [who, name] of [
    ["owner", "speech"],
    ["guest", "speech-clicks"],
  ]) {
    const speech = fixtures.buildFixture(fixtures.fixtureByName(name));
    const room = fixtures.ROOM.room(2 * 48_000, -70);
    const samples = new Float32Array(speech.length + room.length);
    samples.set(speech);
    samples.set(room, speech.length);
    files[who] = join(directory, `${who}.wav`);
    writeFileSync(files[who], wav.encodeWav(samples, 48_000));
  }
  return files;
}

// Recorded in every document before the app runs: the peer connections it opens.
const HOOK = `(() => {
  const probe = (window.__call = { pcs: [] });
  const Native = window.RTCPeerConnection;
  window.RTCPeerConnection = new Proxy(Native, {
    construct(target, args) { const pc = new target(...args); probe.pcs.push(pc); return pc; },
  });
})()`;

const PASSWORD = "Voice-Call-Check-1";
const people = {
  owner: { username: "olga", display: "Olga Owner" },
  guest: { username: "greg", display: "Greg Guest" },
};

const api = (page, method, path, body) =>
  page.eval(`(async () => {
    const headers = { Accept: "application/json", "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0" };
    const server = sessionStorage.getItem("stellarr.active-server");
    if (server) headers["X-Stellarr-Server"] = server;
    const init = { method: ${JSON.stringify(method)}, headers, credentials: "same-origin" };
    ${body === undefined ? "" : `headers["Content-Type"] = "application/json"; init.body = ${JSON.stringify(JSON.stringify(body))};`}
    const response = await fetch(${JSON.stringify(path)}, init);
    return { status: response.status, body: await response.text() };
  })()`);

async function signIn(origin, who, { inviteCode, container }) {
  const { page, username, display } = people[who];
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
            setupCode: setupCode(container),
            initialServer: { name: "Check", description: "", textChannelName: "general", voiceChannelName: "lounge" },
          }),
    });
  }
  if (response.status >= 400) throw new Error(`${who}: sign-in failed ${response.status} ${response.body}`);
  await page.goto(`${origin}/`);
  await page.waitFor(`!!document.querySelector("textarea[aria-label^=Message]")`, 30_000);
  await sleep(800);
}

const voiceSettings = (who) =>
  people[who].page.eval(`(() => {
    const key = Object.keys(localStorage).find((k) => k.startsWith("stellarr.voice.settings.user."));
    return key ? JSON.parse(localStorage.getItem(key)) : "defaults (nothing stored)";
  })()`);

const joinLounge = (who) =>
  people[who].page.eval(`(() => {
    const button = [...document.querySelectorAll("button.channel-main")]
      .find((b) => b.querySelector(".name")?.innerText.trim() === "lounge");
    button?.click();
    return Boolean(button);
  })()`);

/**
 * Every 50 ms for WATCH_MS: the RMS of what arrived from the other person in
 * the last 50 ms, and whether their row in the voice rail is marked speaking.
 */
const watch = (who) =>
  people[who].page.eval(`(async () => {
    const tracks = [];
    for (const pc of window.__call.pcs) {
      if (pc.connectionState === "closed") continue;
      for (const receiver of pc.getReceivers()) {
        if (receiver.track.kind === "audio" && receiver.track.readyState === "live") tracks.push(receiver.track);
      }
    }
    const context = new AudioContext({ sampleRate: 48000 });
    const analyser = context.createAnalyser();
    analyser.fftSize = 2048;
    for (const track of tracks) context.createMediaStreamSource(new MediaStream([track])).connect(analyser);
    // 2048 samples: the last 43 ms, about one sampling step.
    const buffer = new Float32Array(analyser.fftSize);
    const samples = [];
    let peak = 0;
    const started = performance.now();
    while (performance.now() - started < ${WATCH_MS}) {
      await new Promise((r) => setTimeout(r, ${SAMPLE_MS}));
      analyser.getFloatTimeDomainData(buffer);
      let sum = 0;
      for (const v of buffer) { sum += v * v; peak = Math.max(peak, Math.abs(v)); }
      const row = document.querySelector("li.voice-occupant:not(.me)");
      samples.push([Math.sqrt(sum / buffer.length), row ? row.classList.contains("speaking") : null]);
    }
    await context.close();
    return { tracks: tracks.length, peak, samples };
  })()`);

const rtp = (who) =>
  people[who].page.eval(`(async () => {
    let inPackets = 0, outPackets = 0;
    for (const pc of window.__call.pcs) {
      if (pc.connectionState === "closed") continue;
      (await pc.getStats()).forEach((s) => {
        if (s.type === "inbound-rtp" && s.kind === "audio") inPackets += s.packetsReceived;
        if (s.type === "outbound-rtp" && s.kind === "audio") outPackets += s.packetsSent;
      });
    }
    return { inPackets, outPackets };
  })()`);

/** How well the ring at one receiver follows the sound that receiver gets. */
function ringVerdict({ samples }) {
  const sound = samples.filter(([rms]) => rms > SOUND_RMS);
  const ringSeen = samples.some(([, ring]) => ring !== null);
  const caught = sound.length === 0 ? 0 : sound.filter(([, ring]) => ring).length / sound.length;
  let starts = 0;
  let offInPause = 0;
  let onAfterSilence = 0;
  let silentFor = 0;
  let previous = false;
  for (const [rms, ring] of samples) {
    silentFor = rms > SOUND_RMS ? 0 : silentFor + SAMPLE_MS;
    if (ring && !previous) starts += 1;
    if (!ring && rms <= SOUND_RMS) offInPause += 1;
    if (ring && silentFor > 1000) onAfterSilence += 1;
    previous = Boolean(ring);
  }
  const problems = [];
  if (!ringSeen) problems.push("no row for the other person in the voice rail");
  if (caught < 0.8) problems.push(`ring on for only ${(caught * 100).toFixed(0)}% of the time sound arrived`);
  if (offInPause === 0) problems.push("ring never went out in a pause");
  if (starts < 2) problems.push(`ring came on ${starts} time(s), not once per phrase`);
  if (onAfterSilence > 0) problems.push(`ring on after more than a second of silence (${onAfterSilence} samples)`);
  return {
    soundShare: +(sound.length / samples.length).toFixed(2),
    ringOnDuringSound: +caught.toFixed(2),
    ringStarts: starts,
    ringOffInPauses: offInPause,
    problems,
  };
}

const options = parseArgs(process.argv.slice(2));
const work = mkdtempSync(join(tmpdir(), "stellarr-voice-call-"));
const result = { origin: options.origin, at: new Date().toISOString() };
try {
  const files =
    options.owner && options.guest ? { owner: options.owner, guest: options.guest } : await microphoneFiles(work);
  result.microphones = files;
  // chrome.sh adds the flags a call needs to whichever Chrome cdp.mjs finds.
  process.env.STELLARR_CALL_CHROME = findChromium();
  process.env.STELLARR_CHROMIUM = join(here, "chrome.sh");
  for (const who of Object.keys(people)) {
    process.env.STELLARR_CALL_MICROPHONE = files[who];
    const browser = await launch({ headless: true, width: 1280, height: 860 });
    const page = await browser.newPage();
    await page.cmd("Page.addScriptToEvaluateOnNewDocument", { source: HOOK });
    await page.goto(`${options.origin}/login`);
    Object.assign(people[who], { browser, page });
  }

  // The owner registers with a server of their own and invites the guest.
  await signIn(options.origin, "owner", { container: options.container });
  const invite = await api(people.owner.page, "POST", "/api/admin/invites");
  const code = JSON.parse(invite.body).url?.split("/").pop();
  await signIn(options.origin, "guest", { inviteCode: code });

  for (const who of Object.keys(people)) {
    result[who] = { settings: await voiceSettings(who) };
    if (!(await joinLounge(who))) throw new Error(`${who}: no voice channel to join`);
    await sleep(3000);
  }
  await sleep(4000);

  const before = { owner: await rtp("owner"), guest: await rtp("guest") };
  const [ownerHeard, guestHeard] = await Promise.all([watch("owner"), watch("guest")]);
  const after = { owner: await rtp("owner"), guest: await rtp("guest") };
  for (const [who, heard] of [
    ["owner", ownerHeard],
    ["guest", guestHeard],
  ]) {
    result[who] = {
      ...result[who],
      receivedTracks: heard.tracks,
      decodedPeak: +heard.peak.toFixed(3),
      inboundPackets: after[who].inPackets - before[who].inPackets,
      outboundPackets: after[who].outPackets - before[who].outPackets,
      ring: ringVerdict(heard),
    };
  }
  result.problems = Object.keys(people).flatMap((who) => [
    ...(result[who].decodedPeak > PASS_PEAK ? [] : [`${who} decoded silence (peak ${result[who].decodedPeak})`]),
    ...result[who].ring.problems.map((problem) => `${who}: ${problem}`),
  ]);
} catch (error) {
  result.problems = [String(error?.stack ?? error)];
} finally {
  for (const who of Object.keys(people)) {
    try {
      await people[who].browser?.close();
    } catch {
      // The profile directory is in the temporary folder.
    }
  }
  rmSync(work, { recursive: true, force: true });
}

result.pass = result.problems.length === 0;
if (options.out) writeFileSync(options.out, `${JSON.stringify(result, null, 1)}\n`);
for (const who of Object.keys(people)) {
  const r = result[who];
  if (!r?.ring) continue;
  console.log(
    `${who} hears: peak ${r.decodedPeak}, ${r.inboundPackets} packets in; ring on for ${Math.round(r.ring.ringOnDuringSound * 100)}% of the sound, ${r.ring.ringStarts} starts, out in pauses ${r.ring.ringOffInPauses} samples`,
  );
}
for (const problem of result.problems) console.log(`FAIL ${problem}`);
console.log(result.pass ? "PASS" : "FAIL");
// Vite's module runner, which made the microphone files, keeps the process alive.
process.exit(result.pass ? 0 : 1);
