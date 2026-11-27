// Proves Stellarr Media against real browsers.
//
// The script starts the release binary on spare loopback ports, plays the
// hub between it and headless Chromium pages, and runs one scenario per
// behaviour of docs/stellarr-media.md that a str0m client cannot show. Every
// claim about media is read from getStats() or from the decoded media itself,
// never from a connection state.
//
//   node smoke.mjs            every scenario
//   node smoke.mjs a e        only these
//   node smoke.mjs xa xc      encrypted-call scenarios, which run only when named
//
// STELLARR_SMOKE_E2EE=1 runs every scenario as an encrypted call, and
// STELLARR_SMOKE_ENGINE=firefox or webkit runs them in that engine.
//
// It prints one PASS or FAIL line per scenario and exits non-zero when any
// failed. The media server log, the descriptions every page exchanged and the
// numbers behind each line are written to the output directory it names.

import { execFileSync } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { loadavg, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { Client, browserPath, engine, launch, servePage } from "./lib/browser.mjs";
import { nearestColour, nearestTone, writeFixtures } from "./lib/fixtures.mjs";
import { Hub, startMedia } from "./lib/media.mjs";
import { LossyNat, TcpOnlyMediaRoute, isRtp } from "./lib/nat.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

class Failure extends Error {}

function check(condition, message) {
  if (!condition) throw new Failure(message);
}

async function waitFor(what, probe, timeout = 15_000, interval = 200) {
  const end = Date.now() + timeout;
  for (;;) {
    const value = await probe();
    if (value) return value;
    if (Date.now() > end) throw new Failure(`timed out after ${timeout} ms waiting for ${what}`);
    await sleep(interval);
  }
}

// --------------------------------------------------------- encrypted calls

/** An epoch as the encryption client holds it: a sequence, an id and 32 random bytes. */
function newEpoch(sequence) {
  return { sequence, epochId: randomUUID(), key: randomBytes(32).toString("base64") };
}

/** Runs every scenario encrypted with STELLARR_SMOKE_E2EE=1, or by Chromium's older API with =streams. */
function e2eeFromEnv() {
  const value = process.env.STELLARR_SMOKE_E2EE;
  if (!value) return null;
  if (value === "1") return {};
  if (value === "streams") return { path: "streams" };
  throw new Error(`STELLARR_SMOKE_E2EE=${value} is not 1 or streams`);
}

function transformOf(stats, label) {
  return stats?.transforms?.find((t) => t.label === label) ?? null;
}

function droppedOf(transform, ...reasons) {
  if (!transform) return null;
  const counted = reasons.length === 0 ? Object.keys(transform.dropped) : reasons;
  return counted.reduce((total, reason) => total + (transform.dropped[reason] ?? 0), 0);
}

/** CPU seconds used so far by the browser that plays `identity`, with every process under it. */
function browserCpuSeconds(identity) {
  const rows = execFileSync("ps", ["-axo", "pid=,ppid=,time=,command="], { encoding: "utf8" })
    .split("\n")
    .map((line) => line.trim().match(/^(\d+)\s+(\d+)\s+(\S+)\s+(.*)$/))
    .filter(Boolean)
    .map(([, pid, ppid, time, command]) => ({ pid: Number(pid), ppid: Number(ppid), time, command }));
  const seconds = (time) =>
    time
      .split(":")
      .map(Number)
      .reduce((total, part) => total * 60 + part, 0);
  const root = rows.find(
    (r) => r.command.includes(`fake-video-capture=${identity.video}`) && !r.command.includes("--type="),
  );
  if (!root) return null;
  const tree = new Set([root.pid]);
  for (let grew = true; grew; ) {
    grew = false;
    for (const r of rows) {
      if (!tree.has(r.pid) && tree.has(r.ppid)) {
        tree.add(r.pid);
        grew = true;
      }
    }
  }
  return rows.filter((r) => tree.has(r.pid)).reduce((total, r) => total + seconds(r.time), 0);
}

/** The one-minute load average, so a run on a busy machine can be told apart. */
function loadAverage() {
  return Number(loadavg()[0].toFixed(2));
}

// ------------------------------------------------------------------ checks

/** What `receiver` should receive: every publication of everybody else still in. */
function expectedTracks(receiver, clients) {
  return clients
    .filter((c) => c !== receiver && !c.left)
    .flatMap((c) => c.publications.map((p) => ({ publisher: c, publication: p })));
}

/**
 * Waits until the receiver's directory names every track it should receive,
 * each by its publisher's account and current stream id, and nothing else,
 * and every one of those sections is negotiated as receiving.
 */
async function directoryReady(receiver, clients) {
  return waitFor(`the directory of ${receiver.name}`, async () => {
    const snapshot = await receiver.snapshot();
    const wanted = expectedTracks(receiver, clients);
    if (snapshot.directory.length !== wanted.length) return null;
    const mapping = [];
    for (const w of wanted) {
      const entry = snapshot.directory.find(
        (t) =>
          t.userId === w.publisher.userId &&
          t.kind === w.publication.kind &&
          t.streamId === w.publication.streamId,
      );
      if (!entry) return null;
      const transceiver = snapshot.transceivers.find((t) => t.mid === entry.mid);
      if (!transceiver || transceiver.currentDirection !== "recvonly") return null;
      mapping.push({ ...w, mid: entry.mid });
    }
    return { mapping, snapshot };
  });
}

/** Waits until packets, and for video decoded frames, arrived on every section. */
async function firstMedia(receiver, mapping) {
  return waitFor(`media on every section of ${receiver.name}`, async () => {
    const stats = await receiver.stats();
    return mapping.every((m) => {
      const s = stats.inbound[m.mid];
      if (!s || s.packetsReceived === 0) return false;
      return m.publication.kind !== "video" || s.framesDecoded > 0;
    })
      ? stats
      : null;
  });
}

/**
 * Checks everything a seated person must see: the directory, packets and
 * frames advancing on every section, the right person's tone or colour on
 * each, nothing of its own, its own tracks going out, and the selected
 * candidate pair ending at the media server.
 */
async function verifyReceiver(env, receiver, clients) {
  const { mapping, snapshot } = await directoryReady(receiver, clients);
  check(
    snapshot.directory.every((t) => t.userId !== receiver.userId),
    `${receiver.name} has its own track in its directory`,
  );
  await firstMedia(receiver, mapping);

  const before = await receiver.stats();
  await sleep(1500);
  const after = await receiver.stats();
  const measured = await receiver.measure();

  const rows = mapping.map((m) => {
    const b = before.inbound[m.mid] ?? {};
    const a = after.inbound[m.mid] ?? {};
    const row = {
      from: m.publisher.name,
      track: m.publication.name,
      kind: m.publication.kind,
      mid: m.mid,
      packets: [b.packetsReceived ?? 0, a.packetsReceived ?? 0],
      bytes: [b.bytesReceived ?? 0, a.bytesReceived ?? 0],
      codec: a.codec,
    };
    check(
      row.packets[1] > row.packets[0],
      `${receiver.name} mid ${m.mid} (${m.publisher.name} ${row.kind}): packets did not advance ${row.packets}`,
    );
    const media = measured[m.mid];
    if (row.kind === "video") {
      row.frames = [b.framesDecoded ?? 0, a.framesDecoded ?? 0];
      row.keyFrames = a.keyFramesDecoded;
      check(
        row.frames[1] > row.frames[0],
        `${receiver.name} mid ${m.mid}: frames did not advance ${row.frames}`,
      );
      const colour = media?.rgb ? nearestColour(media.rgb) : null;
      row.colour = media?.rgb;
      check(
        colour && colour.name === m.publisher.name && colour.distance < 100,
        `${receiver.name} mid ${m.mid}: the picture ${JSON.stringify(media?.rgb)} is not ${m.publisher.name}'s`,
      );
    } else {
      const tone = media ? nearestTone(media.peakHz) : null;
      row.toneHz = media && Math.round(media.peakHz);
      check(
        tone && tone.name === m.publisher.name && tone.error < 30 && media.peakDb > -90,
        `${receiver.name} mid ${m.mid}: the sound (${row.toneHz} Hz, ${media?.peakDb} dB) is not ${m.publisher.name}'s`,
      );
    }
    return row;
  });

  // Nothing arrives on a section the directory does not name, which is where
  // an own track would show up.
  const known = new Set(mapping.map((m) => m.mid));
  for (const [mid, b] of Object.entries(before.inbound)) {
    const a = after.inbound[mid];
    check(
      known.has(mid) || !a || a.packetsReceived === b.packetsReceived,
      `${receiver.name} receives packets on mid ${mid}, which its directory does not name`,
    );
  }

  const sending = receiver.publications.map((p) => {
    const b = before.outbound[p.mid] ?? {};
    const a = after.outbound[p.mid] ?? {};
    check(
      (a.packetsSent ?? 0) > (b.packetsSent ?? 0),
      `${receiver.name} does not send its ${p.name} (mid ${p.mid})`,
    );
    return {
      name: p.name,
      mid: p.mid,
      packetsSent: [b.packetsSent ?? 0, a.packetsSent],
      bytesSent: [b.bytesSent ?? 0, a.bytesSent],
      kbps: Math.round(((a.bytesSent - (b.bytesSent ?? 0)) * 8) / 1500),
      targetKbps: Math.round(a.targetBitrate / 1000),
      codec: a.codec,
    };
  });

  const pair = after.pair;
  check(pair, `${receiver.name} has no selected candidate pair`);
  // WebKit leaves the address out of its statistics; the port still tells.
  const remote = `${pair.remote.address ?? env.candidate.split(":")[0]}:${pair.remote.port}`;
  check(
    remote === env.candidate,
    `${receiver.name}'s selected pair ends at ${remote}, not at the media server ${env.candidate}`,
  );

  return { name: receiver.name, rows, sending, pair };
}

async function verifyRoom(env, clients) {
  const live = clients.filter((c) => !c.left);
  return Promise.all(live.map((c) => verifyReceiver(env, c, clients)));
}

function summarise(results) {
  return results
    .map((r) => {
      const rows = r.rows
        .map((row) => {
          const what = row.kind === "video" ? `${row.frames[1]}f` : `${row.toneHz}Hz`;
          return `${row.from}.${row.kind[0]}@${row.mid}:${row.packets[1]}p/${what}`;
        })
        .join(" ");
      return `${r.name}<-[${rows}]`;
    })
    .join(" ");
}


// ------------------------------------------------------------ poor links

/** Chromium's payload type for Opus, in both directions. */
const OPUS_PT = 111;

/** The poor downlinks of scenario q, slowest last. */
const CAPS_KBPS = [512, 256, 128];
/** How long a cap is held before it is measured, and how long it is measured. */
const SETTLE_MS = 10_000;
const WINDOW_MS = 10_000;

/** A poor downlink of `kbps`: 300 ms of queue, 40 ms of delay, 15 ms of jitter, 1% lost in bursts of 3. */
function poorLink(kbps) {
  return { kbps, queueMs: 300, delayMs: 40, jitterMs: 15, lossMean: 0.01, lossBurst: 3, seed: kbps };
}

/**
 * Runs `body` against a second media server that the browsers reach only
 * through a relay, which can shape the downlink of one of them.
 */
async function relayed(s, id, body) {
  const env = s.env;
  const nat = await LossyNat.listen();
  const logFile = join(env.out, `stellarr-media-${id}.log`);
  const media = await startMedia({ binary: env.binary, logFile, advertisePort: nat.port });
  nat.target = media.port;
  const hub = new Hub(media.control);
  await hub.start();
  const inner = new Scenario(
    { ...env, hub, candidate: media.candidate, logText: () => readFileSync(logFile, "utf8") },
    id,
  );
  try {
    const result = await body(inner, nat);
    await inner.verifyClean();
    return result;
  } finally {
    await inner.dump().catch(() => {});
    await inner.cleanup();
    await hub.stop();
    await media.stop();
    nat.close();
  }
}

/** Seats `name` and waits until the relay has heard from it; returns its addresses there. */
async function joinThrough(s, nat, name, options) {
  const known = new Set(nat.addresses());
  const client = await s.join(name, options);
  await waitFor(`${name} to reach the relay`, async () => {
    const stats = await client.stats();
    return stats.pair?.state === "succeeded" && nat.addresses().some((a) => !known.has(a));
  });
  return { client, addresses: nat.addresses().filter((a) => !known.has(a)) };
}

/** A camera that costs a real bitrate, in the person's colour. */
function busyCamera(client, maxBitrate, size = {}) {
  return client.publish("camera", "video", { busy: { rgb: client.identity.rgb, ...size }, maxBitrate });
}

/**
 * What one receiver got between two readings of its statistics: for every
 * voice the share of packets lost and of samples concealed, the delay of its
 * jitter buffer and whose tone it carries; for every picture the frames
 * decoded; and the bitrate received.
 */
function receivedBetween(before, after, measured, mapping, ms) {
  const seconds = ms / 1000;
  const voices = [];
  const pictures = [];
  let bytes = 0;
  for (const m of mapping) {
    const b = before.inbound[m.mid] ?? {};
    const a = after.inbound[m.mid] ?? {};
    const delta = (field) => (a[field] ?? 0) - (b[field] ?? 0);
    bytes += delta("bytesReceived");
    if (m.publication.kind === "audio") {
      const lost = delta("packetsLost");
      const received = delta("packetsReceived");
      const samples = delta("totalSamplesReceived");
      const emitted = delta("jitterBufferEmittedCount");
      const tone = measured[m.mid] ? nearestTone(measured[m.mid].peakHz) : null;
      voices.push({
        from: m.publisher.name,
        mid: m.mid,
        lostPct: lost + received > 0 ? (100 * lost) / (lost + received) : 100,
        concealedPct: samples > 0 ? (100 * delta("concealedSamples")) / samples : 100,
        concealmentEvents: delta("concealmentEvents"),
        jitterBufferMs: emitted > 0 ? (1000 * delta("jitterBufferDelay")) / emitted : null,
        tone: tone && tone.name === m.publisher.name && tone.error < 30 && measured[m.mid].peakDb > -90,
      });
    } else {
      pictures.push({
        from: m.publisher.name,
        mid: m.mid,
        fps: delta("framesDecoded") / seconds,
        kbps: (delta("bytesReceived") * 8) / seconds / 1000,
        freezes: delta("freezeCount"),
        keyFrames: delta("keyFramesDecoded"),
      });
    }
  }
  return { voices, pictures, kbps: (bytes * 8) / seconds / 1000 };
}

function describeWindow(w) {
  const voices = w.voices
    .map(
      (v) =>
        `${v.from} lost ${v.lostPct.toFixed(1)}% concealed ${v.concealedPct.toFixed(1)}% ` +
        `jb ${v.jitterBufferMs === null ? "-" : Math.round(v.jitterBufferMs)} ms${v.tone ? "" : " NO TONE"}`,
    )
    .join(", ");
  const pictures = w.pictures
    .map((p) => `${p.from} ${p.fps.toFixed(1)} fps ${Math.round(p.kbps)} kbit/s${p.held ? " held" : ""}`)
    .join(", ");
  const queueing = w.link?.voiceQueueing
    ? `, voice queued p50 ${Math.round(w.link.voiceQueueing.p50)} ms p95 ${Math.round(w.link.voiceQueueing.p95)} ms`
    : "";
  const link =
    w.link === undefined
      ? ""
      : `, link ${Math.round(w.link.kbps)} kbit/s (queue drops ${w.link.queueDrops}, burst drops ${w.link.burstDrops}${queueing})`;
  return `[${voices}; ${pictures}; received ${Math.round(w.kbps)} kbit/s${link}]`;
}

/** What the media server says of one session's downlink, in kbit/s. */
async function downlinkOf(hub, sessionId) {
  const stats = await hub.control.request("GET", "/v1/stats");
  const session = stats.body.rooms.flatMap((r) => r.sessions).find((x) => x.sessionId === sessionId);
  const d = session?.downlink;
  if (!d) return null;
  const k = (bps) => (bps === null || bps === undefined ? null : Math.round(bps / 1000));
  return {
    estimate: k(d.estimateBps),
    raw: k(d.rawEstimateBps),
    smoothed: k(d.smoothedEstimateBps),
    forwarded: k(d.forwardedBps),
    padding: k(d.paddingBps),
    held: d.heldBack.map((h) => h.mid).sort(),
    holds: d.holds,
    resumes: d.resumes,
  };
}

/** What a shaped link did since `before`, as a rate of whole datagrams. */
function linkBetween(before, after, ms, voiceQueueingMs = []) {
  return {
    kbps: ((after.deliveredBytes - before.deliveredBytes) * 8) / ms,
    queueDrops: after.queueDrops - before.queueDrops,
    burstDrops: after.burstDrops - before.burstDrops,
    voiceQueueing: voiceQueueingMs.length > 0 ? percentiles(voiceQueueingMs) : null,
  };
}

/** Percentiles of a list of numbers. */
function percentiles(values) {
  const sorted = [...values].sort((x, y) => x - y);
  const at = (p) => sorted[Math.min(sorted.length - 1, Math.round((sorted.length - 1) * p))];
  return { n: sorted.length, p50: at(0.5), p95: at(0.95), max: sorted[sorted.length - 1] };
}

/**
 * Records when every Opus packet passes the relay on its way up from
 * `fromAddresses` and on its way down to `toAddresses`, by RTP timestamp,
 * which the media server keeps. The difference is the time a voice spends in
 * the media server.
 */
function voiceTransit(nat, fromAddresses, toAddresses) {
  const up = new Map();
  const transits = [];
  const tap = (direction, address, data, at) => {
    if (!isRtp(data, OPUS_PT)) return;
    const ts = data.readUInt32BE(4);
    if (direction === "up" && fromAddresses.includes(address)) up.set(ts, at);
    if (direction === "down" && toAddresses.includes(address) && up.has(ts)) {
      transits.push(at - up.get(ts));
      up.delete(ts);
    }
  };
  nat.taps.push(tap);
  return {
    stop() {
      nat.taps = nat.taps.filter((t) => t !== tap);
      return percentiles(transits);
    },
  };
}

// --------------------------------------------------------------- scenarios

class Scenario {
  constructor(env, id) {
    this.env = env;
    this.id = id;
    this.room = randomUUID();
    // The epoch every person of an encrypted room holds, as the conversation's
    // epoch envelopes would give it to them.
    this.epoch = newEpoch(1);
    this.clients = [];
    this.allowIgnoredOffers = false;
    this.tolerated = new Set();
    this.logFrom = env.logText().length;
  }

  /**
   * Seats a person. A tunes Opus in its descriptions like the web client.
   * `e2ee` encrypts this person's frames: undefined takes the run's
   * (STELLARR_SMOKE_E2EE), null none, and an object is laid over the room's
   * epoch. `cbr` asks A's voice encoder for a constant bitrate.
   */
  async join(name, { mic = true, via, e2ee, cbr = false, initScript = null, pattern } = {}) {
    const identity = this.env.identities.find((i) => i.name === name);
    const wanted = e2ee === undefined ? (this.env.e2ee ? { ...this.env.e2ee } : null) : e2ee;
    const client = await Client.open({
      browser: this.env.browsers[name],
      url: this.env.url,
      hub: this.env.hub,
      identity,
      roomId: this.room,
      userId: randomUUID(),
      tuneOpus: name === "A",
      cbr,
      initScript,
      e2ee:
        wanted === null
          ? null
          : { conversationId: this.room, epochs: [this.epoch], send: this.epoch.sequence, ...wanted },
    });
    if (pattern) {
      this.clients.push(client);
      if (mic) await client.publish("mic", "audio", { pattern });
      return client;
    }
    this.clients.push(client);
    if (mic) await client.publish("mic", "audio", via ? { via } : {});
    return client;
  }

  sessionIds() {
    return new Set(this.clients.map((c) => c.sessionId));
  }

  log() {
    return this.env.logText().slice(this.logFrom);
  }

  /**
   * What must hold after any scenario, except for sessions the scenario
   * breaks on purpose and checks itself.
   */
  async verifyClean() {
    const ids = this.sessionIds();
    for (const id of this.tolerated) ids.delete(id);
    const hub = this.env.hub;
    const closed = hub.events.filter((e) => ids.has(e.sessionId) && e.type === "closed");
    check(closed.length === 0, `the media server closed sessions: ${JSON.stringify(closed)}`);
    const refused = hub.calls.filter((c) => ids.has(c.sessionId) && c.status !== 204);
    check(refused.length === 0, `control API calls failed: ${JSON.stringify(refused)}`);

    for (const client of this.clients) {
      if (this.tolerated.has(client.sessionId)) continue;
      check(client.pageErrors.length === 0, `${client.name}: ${client.pageErrors.join("; ")}`);
      if (client.left) continue;
      const snapshot = await client.snapshot();
      check(snapshot.errors.length === 0, `${client.name}: ${snapshot.errors.join("; ")}`);
      check(
        this.allowIgnoredOffers || snapshot.offersIgnored.length === 0,
        `${client.name} ignored offers ${snapshot.offersIgnored}`,
      );
    }

    // str0m's own warnings name no session, so every warning counts.
    const lines = this.log().split("\n");
    const mine = lines.filter((line) => [...ids].some((id) => line.includes(id)));
    const bad = [
      ...mine.filter((line) => /went unanswered|panicked|refused a|connection failed/.test(line)),
      ...lines
        .filter((line) => / (WARN|ERROR) /.test(line))
        .filter((line) => ![...this.tolerated].some((id) => line.includes(id))),
    ];
    check(bad.length === 0, `the media server log has: ${[...new Set(bad)].join(" | ")}`);
  }

  async dump() {
    for (const client of this.clients) {
      if (client.left && !client.savedDescriptions) continue;
      const descriptions = client.savedDescriptions ?? (await client.descriptions());
      writeFileSync(
        join(this.env.out, `${this.id}-${client.name}-descriptions.json`),
        JSON.stringify(descriptions, null, 2),
      );
    }
  }

  async cleanup() {
    for (const client of this.clients) await client.leave().catch(() => {});
  }
}

/** How many sections the last offer of the media server to a page has. */
async function mLinesOfOffer(client) {
  const offer = (await client.descriptions())
    .filter((d) => d.side === "remote" && d.type === "offer")
    .pop();
  return offer.sdp.split("\r\nm=").length - 1;
}

/** Waits until a page has the section `mid` inactive and out of its directory. */
function sectionFreed(client, mid) {
  return waitFor(`${client.name}'s section ${mid} to go inactive`, async () => {
    const snapshot = await client.snapshot();
    const t = snapshot.transceivers.find((x) => x.mid === mid);
    return t?.currentDirection === "inactive" && !snapshot.directory.some((e) => e.mid === mid);
  });
}

/**
 * What a page shows of a section another publisher's track took over: a new
 * SSRC, and the browser filing the track it already had under the stream the
 * new publisher declared.
 */
async function checkTakenOver(client, mid, statsBefore, streamId) {
  const statsAfter = (await client.stats()).inbound[mid];
  check(
    statsAfter.ssrc !== statsBefore.ssrc,
    `${client.name}'s section ${mid} kept the SSRC ${statsBefore.ssrc}`,
  );
  const arrivals = (await client.snapshot()).arrivals.filter((a) => a.mid === mid);
  check(
    arrivals.at(-1).streams.includes(streamId),
    `${client.name}'s last track event on ${mid} names ${arrivals.at(-1).streams}, not ${streamId}`,
  );
  return (
    `SSRC ${statsBefore.ssrc} -> ${statsAfter.ssrc}, ${arrivals.length} track events ` +
    `(${new Set(arrivals.map((a) => a.trackId)).size} tracks)`
  );
}

/** Saves a page's descriptions before it leaves, for the dump. */
async function leave(client) {
  client.savedDescriptions = await client.descriptions();
  await client.leave();
}

async function verifyDirectMedia(s, protocol, withVideo = false) {
  const first = await s.join("A");
  const second = await s.join("B");
  if (withVideo) {
    await first.publish("camera", "video");
    await directoryReady(second, s.clients);
    await second.publish("camera", "video");
  }
  const results = await verifyRoom(s.env, s.clients);
  for (const result of results) {
    const { local, remote } = result.pair;
    check(local?.protocol === protocol && remote?.protocol === protocol,
      `${result.name} selected ${local?.protocol}/${remote?.protocol}, expected ${protocol}`);
    check(local.candidateType !== "relay" && remote.candidateType === "host",
      `${result.name} did not connect directly to the SFU candidate`);
    check(result.rows.some((row) => row.kind === "audio"), `${result.name} received no audio track`);
    if (withVideo) {
      check(result.rows.some((row) => row.kind === "video"), `${result.name} received no video track`);
    }
    for (const incoming of result.rows) {
      check(incoming.bytes[1] > incoming.bytes[0], `${result.name} did not receive ${incoming.kind} bytes`);
    }
    for (const outgoing of result.sending) {
      check(outgoing.bytesSent[1] > outgoing.bytesSent[0], `${result.name} did not send ${outgoing.name} bytes`);
    }
  }
  return { summary: `${protocol.toUpperCase()} ${summarise(results)}`, results };
}

const SCENARIOS = {
  /** Direct bidirectional audio with normal UDP connectivity and no relay. */
  async udp(s) {
    return verifyDirectMedia(s, "udp");
  },

  /** The same two browsers, with a TCP-only network path to the advertised port. */
  async tcp(s) {
    const route = await TcpOnlyMediaRoute.listen();
    let media;
    let hub;
    let scenario;
    try {
      const logFile = join(s.env.out, "tcp-media.log");
      media = await startMedia({ binary: s.env.binary, logFile, advertisePort: route.port });
      route.target = media.port;
      hub = new Hub(media.control);
      await hub.start();
      scenario = new Scenario({
        ...s.env,
        hub,
        candidate: media.candidate,
        logText: () => readFileSync(logFile, "utf8"),
      }, s.id);
      const report = await verifyDirectMedia(scenario, "tcp", true);
      for (const client of scenario.clients) {
        const descriptions = await client.descriptions();
        check(descriptions.some(({ side, sdp }) => side === "remote" &&
          sdp.split(/\r?\n/).some((line) => /^a=candidate:.*\btcp\b.*\btyp host tcptype passive\b/i.test(line))),
        `${client.name} was not advertised a passive SFU TCP candidate`);
      }
      check(route.accepted >= 2, `TCP route accepted only ${route.accepted} connections`);
      check(route.dropped > 0, "the UDP blackhole received no connectivity checks");
      await scenario.verifyClean();
      check(hub.resets === 0 && !hub.pumpError, `TCP hub polling failed: ${hub.pumpError ?? hub.resets}`);
      return {
        ...report,
        summary: `${report.summary}; ${route.accepted} TCP connections, ${route.dropped} UDP datagrams discarded`,
        route: { tcpConnections: route.accepted, droppedUdpDatagrams: route.dropped },
      };
    } finally {
      await scenario?.dump().catch((error) => console.log(`dump of tcp failed: ${error}`));
      await scenario?.cleanup();
      await hub?.stop();
      await media?.stop();
      await route.close();
    }
  },


  /** Two people with a fake microphone each hear each other and nothing of their own. */
  async a(s) {
    await s.join("A");
    await s.join("B");
    const results = await verifyRoom(s.env, s.clients);

    // The audio level header extension reaches the receivers.
    const levels = [];
    for (const r of results) {
      const client = s.clients.find((c) => c.name === r.name);
      const measured = await client.levels();
      for (const row of r.rows) {
        const level = measured[row.mid]?.[0];
        check(
          typeof level === "number" && level > 0,
          `${r.name} mid ${row.mid} carries no audio level`,
        );
        levels.push(`${r.name}@${row.mid} level ${level.toFixed(2)}`);
      }
    }
    return { summary: `${summarise(results)}; ${levels.join(", ")}`, results };
  },

  /**
   * A camera that has been running alone for seconds is decoded by a person
   * who arrives afterwards. Its encoder's own first keyframe is long gone, so
   * the picture starts only because the media server asked for a keyframe.
   */
  async b(s) {
    const A = await s.join("A");
    const camera = await A.publish("camera", "video");
    await sleep(4000);
    const alone = (await A.stats()).outbound[camera.mid];

    const joined = Date.now();
    const B = await s.join("B");
    const { mapping } = await directoryReady(B, s.clients);
    const mid = mapping.find((m) => m.publication.kind === "video").mid;
    await waitFor("B's first decoded frame", async () => {
      const stats = await B.stats();
      return stats.inbound[mid]?.framesDecoded > 0;
    });
    const firstFrameMs = Date.now() - joined;

    const results = await verifyRoom(s.env, s.clients);
    const received = (await B.stats()).inbound[mid];
    const sent = (await A.stats()).outbound[camera.mid];
    check(received.keyFramesDecoded >= 1, `B decoded no keyframe on ${mid}`);
    check(
      sent.pliCount + sent.firCount > alone.pliCount + alone.firCount,
      `A was not asked for a keyframe (PLI ${alone.pliCount} -> ${sent.pliCount}, FIR ${alone.firCount} -> ${sent.firCount})`,
    );
    check(
      sent.keyFramesEncoded > alone.keyFramesEncoded,
      `A encoded no keyframe after B arrived (${alone.keyFramesEncoded} -> ${sent.keyFramesEncoded})`,
    );
    return {
      summary:
        `first frame ${firstFrameMs} ms after joining; A PLI ${alone.pliCount}->${sent.pliCount}, ` +
        `keyframes encoded ${alone.keyFramesEncoded}->${sent.keyFramesEncoded}; ` +
        `B keyframes decoded ${received.keyFramesDecoded}, codec ${received.codec}; ${summarise(results)}`,
      results,
    };
  },

  /**
   * A third person arrives late and receives every existing track, each
   * filed under the person whose tone or colour it carries.
   */
  async c(s) {
    const A = await s.join("A");
    await A.publish("camera", "video");
    const B = await s.join("B");
    await B.publish("camera", "video");
    await verifyRoom(s.env, s.clients);
    await sleep(1000);

    const C = await s.join("C");
    const results = await verifyRoom(s.env, s.clients);
    const forC = results.find((r) => r.name === "C");
    check(forC.rows.length === 4, `C receives ${forC.rows.length} tracks instead of 4`);

    const snapshot = await C.snapshot();
    const late = snapshot.arrivals.filter((a) => !a.directoryKnown);
    check(
      late.length === 0,
      `C's tracks arrived before their directory entry: ${JSON.stringify(late)}`,
    );
    const arrivals = snapshot.arrivals.map((a) => ({
      mid: a.mid,
      kind: a.kind,
      streams: a.streams,
      streamIsPublishers: s.clients.some((c) =>
        c.publications.some((p) => a.streams.includes(p.streamId)),
      ),
      signalingState: a.signalingState,
      mutedAtArrival: a.mutedAtArrival,
      trackEvents: a.events.map((e) => e.name),
    }));
    return {
      summary:
        `${summarise([forC])}; ontrack ${JSON.stringify(arrivals)}; ` +
        `negotiationneeded fired ${snapshot.negotiationNeeded} times for 1 publication`,
      results,
      arrivals,
    };
  },

  /**
   * A person leaves: the others' sections of that person go inactive and the
   * directory shrinks. A later offer of a remaining browser leaves them so.
   */
  async d(s) {
    const A = await s.join("A");
    const B = await s.join("B");
    await B.publish("camera", "video");
    const C = await s.join("C");
    await verifyRoom(s.env, s.clients);

    const bMids = {};
    for (const r of [A, C]) {
      const snapshot = await r.snapshot();
      bMids[r.name] = snapshot.directory.filter((t) => t.userId === B.userId).map((t) => t.mid);
      check(bMids[r.name].length === 2, `${r.name} does not have both of B's tracks`);
    }

    await leave(B);
    const left = {};
    for (const r of [A, C]) {
      const snapshot = await waitFor(`${r.name}'s sections of B to go inactive`, async () => {
        const snap = await r.snapshot();
        const gone = snap.directory.every((t) => t.userId !== B.userId);
        const inactive = bMids[r.name].every(
          (mid) => snap.transceivers.find((t) => t.mid === mid)?.currentDirection === "inactive",
        );
        return gone && inactive ? snap : null;
      });
      const before = await r.stats();
      await sleep(1000);
      const after = await r.stats();
      for (const mid of bMids[r.name]) {
        const a = after.inbound[mid];
        const b = before.inbound[mid];
        // An inactive section's stats may go away or stand still.
        check(
          !a || (b && a.packetsReceived === b.packetsReceived),
          `${r.name} still receives on ${mid} after B left`,
        );
      }
      left[r.name] = bMids[r.name].map((mid) => {
        const t = snapshot.transceivers.find((x) => x.mid === mid);
        const arrival = snapshot.arrivals.find((x) => x.mid === mid);
        return {
          mid,
          direction: t.direction,
          currentDirection: t.currentDirection,
          muted: t.receiverMuted,
          readyState: t.receiverReadyState,
          trackEvents: arrival?.events.map((e) => e.name),
          statsAfter: after.inbound[mid] ? "present" : "absent",
        };
      });
    }
    await verifyRoom(s.env, s.clients);

    // A browser offer after the leave still lists those sections. They stay
    // out of the directory as B's; one is inactive until a track of its kind
    // is forwarded, and then carries that track and nobody else's.
    await A.publish("camera", "video");
    const results = await verifyRoom(s.env, s.clients);
    for (const r of [A, C]) {
      const snapshot = await r.snapshot();
      for (const mid of bMids[r.name]) {
        const t = snapshot.transceivers.find((x) => x.mid === mid);
        const named = snapshot.directory.find((e) => e.mid === mid);
        check(
          named
            ? named.userId === A.userId && t.currentDirection === "recvonly"
            : t.currentDirection === "inactive",
          `${r.name}'s section ${mid} of B is ${t.currentDirection} after A's offer, ` +
            `and the directory names ${named?.userId ?? "nobody"} on it`,
        );
      }
    }
    const offered = (await A.descriptions())
      .filter((d) => d.side === "local" && d.type === "offer")
      .pop()
      .sdp.split("\r\nm=")
      .filter((section) => bMids.A.some((mid) => section.includes(`a=mid:${mid}\r\n`)))
      .map((section) => section.match(/a=(sendrecv|sendonly|recvonly|inactive)/)?.[1]);
    return {
      summary:
        `after the leave ${JSON.stringify(left)}; ` +
        `A's next offer lists B's sections as ${offered}; ${summarise(results)}`,
      results,
      left,
      offeredDirections: offered,
    };
  },

  /**
   * A forced collision: the media server's offer is held back until the page
   * has an offer of its own outstanding. The page ignores the server's offer,
   * the media server rolls its own back, and both changes land.
   */
  async e(s) {
    s.allowIgnoredOffers = true;
    await s.join("A");
    const B = await s.join("B");
    const C = await s.join("C");
    await verifyRoom(s.env, s.clients);
    const hub = s.env.hub;

    hub.hold(C.sessionId);
    await B.publish("camera", "video");
    const held = await waitFor("the media server's offer to C", async () =>
      hub.heldEvents(C.sessionId).find((e) => e.kind === "offer"),
    );
    await C.publish("camera", "video");
    await hub.posted(C.sessionId);
    const offerCall = hub.callsOf(C.sessionId).filter((c) => c.kind === "offer").pop();
    check(offerCall.status === 204, `C's colliding offer got ${offerCall.status}`);
    await hub.release(C.sessionId);

    const results = await verifyRoom(s.env, s.clients);
    const snapshot = await C.snapshot();
    const heldId = held.payload.negotiationId;
    check(
      JSON.stringify(snapshot.offersIgnored) === JSON.stringify([heldId]),
      `C ignored ${JSON.stringify(snapshot.offersIgnored)}, not exactly offer ${heldId}`,
    );
    check(!snapshot.offersAnswered.includes(heldId), `C answered the ignored offer ${heldId}`);
    const later = snapshot.offersAnswered.filter((id) => id > heldId);
    check(later.length > 0, `C answered no offer after ${heldId}`);
    const rolledBack = s
      .log()
      .split("\n")
      .some((line) => line.includes("offer collision") && line.includes(C.sessionId));
    check(rolledBack, "the media server log shows no rollback for C");
    return {
      summary: `C ignored offer ${heldId}, answered ${later}; ${summarise(results)}`,
      results,
    };
  },

  /**
   * A publisher swaps its camera for a capture in a new stream: the others
   * get the new stream id in a tracks update, no offer, and keep the picture.
   */
  async f(s) {
    const A = await s.join("A");
    const camera = await A.publish("camera", "video");
    const B = await s.join("B");
    const C = await s.join("C");
    await verifyRoom(s.env, s.clients);
    const hub = s.env.hub;

    const before = {};
    for (const r of [B, C]) {
      const snapshot = await r.snapshot();
      before[r.name] = {
        mid: snapshot.directory.find((t) => t.userId === A.userId && t.kind === "video").mid,
        offers: hub.signalsOf(r.sessionId, "offer").length,
        tracks: hub.signalsOf(r.sessionId, "tracks").length,
      };
    }

    const swapped = await A.swap("camera");
    check(swapped.streamId !== camera.streamId, "the new capture has the old stream id");
    check(swapped.mid === camera.mid, "the swap moved the camera to another section");

    const after = {};
    for (const r of [B, C]) {
      await waitFor(`the new stream id at ${r.name}`, async () => {
        const snapshot = await r.snapshot();
        const entry = snapshot.directory.find((t) => t.mid === before[r.name].mid);
        return entry?.streamId === swapped.streamId;
      });
      after[r.name] = {
        offers: hub.signalsOf(r.sessionId, "offer").length - before[r.name].offers,
        tracks: hub.signalsOf(r.sessionId, "tracks").length - before[r.name].tracks,
      };
      check(
        after[r.name].offers === 0,
        `${r.name} got ${after[r.name].offers} offers for the swap`,
      );
      check(after[r.name].tracks >= 1, `${r.name} got no tracks update`);
    }

    const results = await verifyRoom(s.env, s.clients);
    const streams = {};
    for (const r of [B, C]) {
      const arrival = (await r.snapshot()).arrivals.find((a) => a.mid === before[r.name].mid);
      streams[r.name] = arrival.streams.map((id) => id.slice(0, 8));
    }
    for (const r of [B, C]) {
      const row = results
        .find((x) => x.name === r.name)
        .rows.find((x) => x.from === "A" && x.kind === "video");
      check(row.mid === before[r.name].mid, `${r.name} receives A's camera on another section`);
    }
    return {
      summary:
        `stream ${camera.streamId.slice(0, 8)} -> ${swapped.streamId.slice(0, 8)}; ` +
        `${JSON.stringify(after)}; the receivers' MediaStream ids stay ${JSON.stringify(streams)}; ` +
        summarise(results),
      results,
    };
  },

  /**
   * Every video codec of the contract travels: a publisher that prefers
   * VP9, H.264, AV1 or VP9 with temporal layers sends it, and a receiver
   * that numbers the codecs its own way decodes it.
   */
  async i(s) {
    const A = await s.join("A");
    const wanted = [
      { name: "vp9", codec: "VP9" },
      { name: "h264", codec: "H264" },
      { name: "av1", codec: "AV1" },
      { name: "vp9-l1t3", codec: "VP9", scalabilityMode: "L1T3" },
    ];
    for (const w of wanted) {
      await A.publish(w.name, "video", { codec: w.codec, scalabilityMode: w.scalabilityMode });
    }
    const B = await s.join("B");
    const results = await verifyRoom(s.env, s.clients);

    const sent = (await A.stats()).outbound;
    const received = results.find((r) => r.name === "B").rows;
    const codecs = wanted.map((w) => {
      const publication = A.publications.find((p) => p.name === w.name);
      const out = sent[publication.mid];
      const row = received.find((r) => r.from === "A" && r.track === w.name);
      check(
        out.codec?.startsWith(`video/${w.codec}`),
        `A sends ${out.codec} for ${w.name}, not ${w.codec}`,
      );
      check(
        row?.codec?.startsWith(`video/${w.codec}`),
        `B decodes ${w.name} as ${row?.codec}, not ${w.codec}`,
      );
      if (w.scalabilityMode) {
        check(
          out.scalabilityMode === w.scalabilityMode,
          `A encodes ${w.name} as ${out.scalabilityMode}`,
        );
      }
      return `${w.name}: ${out.codec} -> ${row.codec} ${row.frames[1]}f`;
    });
    return { summary: codecs.join("; "), results };
  },

  /**
   * A camera turned off and on twice: once with removeTrack, which leaves an
   * inactive section, and once with transceiver.stop(), whose m-line the
   * browser may recycle for the next camera. The receiver's section of the
   * withdrawn camera, once it is inactive, carries the next camera, and the
   * media server's descriptions to it do not grow.
   */
  async k(s) {
    const A = await s.join("A");
    const B = await s.join("B");
    // A's first camera must not meet the offer of B's microphone, or the
    // scenario would be the collision of e by chance.
    await directoryReady(A, s.clients);
    const steps = [];

    const cameraMidAtB = async () => {
      const { mapping } = await directoryReady(B, s.clients);
      return mapping.find((m) => m.publication.kind === "video").mid;
    };
    const mLines = async (client, side) =>
      (await client.descriptions())
        .filter((d) => d.side === side && d.type === "offer")
        .pop()
        .sdp.split("\r\nm=").length - 1;

    const mids = [];
    const offered = [];
    for (const how of ["remove", "stop", null]) {
      const camera = await A.publish("camera", "video");
      await verifyRoom(s.env, s.clients);
      const atB = await cameraMidAtB();
      check(
        mids.every((mid) => mid === atB),
        `B received the new camera on ${atB} and not on the section ${mids[0]} that was freed`,
      );
      mids.push(atB);
      offered.push(await mLines(B, "remote"));
      steps.push(
        `camera at A ${camera.mid}, B ${atB}, A offers ${await mLines(A, "local")} m-lines, ` +
          `the media server ${offered.at(-1)}`,
      );
      if (!how) break;
      await A.unpublish("camera", how);
      await sectionFreed(B, atB);
      steps.push(`${how}: B's ${atB} inactive`);
    }
    check(
      offered.every((n) => n === offered[0]),
      `the media server's offers to B grew: ${offered}`,
    );

    const results = await verifyRoom(s.env, s.clients);
    return { summary: `${steps.join("; ")}; ${summarise(results)}`, results };
  },

  /**
   * A's camera is received by B and stops, and C starts a camera. B decodes
   * C's colour on the section that carried A's camera, which the directory
   * now names as C's, with the stream C declared, and B's descriptions have
   * no more sections than before.
   */
  async s(s) {
    const A = await s.join("A");
    const B = await s.join("B");
    const C = await s.join("C");
    // Neither camera may meet an offer of the media server to its own page,
    // or the scenario would be the collision of e by chance.
    await directoryReady(A, s.clients);
    const camera = await A.publish("camera", "video");
    await verifyRoom(s.env, s.clients);

    const before = await B.snapshot();
    const section = before.directory.find((t) => t.userId === A.userId && t.kind === "video").mid;
    const atC = (await C.snapshot()).directory.find((t) => t.userId === A.userId && t.kind === "video").mid;
    const sectionsBefore = await mLinesOfOffer(B);
    const statsBefore = (await B.stats()).inbound[section];

    await A.unpublish("camera", "remove");
    await sectionFreed(B, section);
    await sectionFreed(C, atC);

    const shown = await C.publish("camera", "video");
    const results = await verifyRoom(s.env, s.clients);

    const snapshot = await B.snapshot();
    const entry = snapshot.directory.find((t) => t.userId === C.userId && t.kind === "video");
    check(entry?.mid === section, `C's camera is on ${entry?.mid} at B, not on the freed ${section}`);
    check(entry.streamId === shown.streamId, `the directory names the stream ${entry.streamId}`);
    check(
      snapshot.directory.every((t) => t.userId !== A.userId || t.kind !== "video"),
      "A's camera is still in B's directory",
    );
    const sectionsAfter = await mLinesOfOffer(B);
    check(
      sectionsAfter === sectionsBefore,
      `B's description has ${sectionsAfter} sections, it had ${sectionsBefore}`,
    );
    const taken = await checkTakenOver(B, section, statsBefore, shown.streamId);
    return {
      summary:
        `B's section ${section}: A ${camera.streamId.slice(0, 8)} -> C ${entry.streamId.slice(0, 8)}, ` +
        `${taken}, ${sectionsAfter} sections; ${summarise(results)}`,
      results,
    };
  },

  /**
   * The same with sound: A's microphone is withdrawn and C joins with its
   * own. B hears C's tone on the section that carried A's, attributed to C.
   */
  async t(s) {
    const A = await s.join("A");
    const B = await s.join("B");
    await verifyRoom(s.env, s.clients);

    const before = await B.snapshot();
    const section = before.directory.find((t) => t.userId === A.userId && t.kind === "audio").mid;
    const sectionsBefore = await mLinesOfOffer(B);
    const statsBefore = (await B.stats()).inbound[section];

    await A.unpublish("mic", "remove");
    await sectionFreed(B, section);

    const C = await s.join("C");
    const results = await verifyRoom(s.env, s.clients);

    const snapshot = await B.snapshot();
    const entry = snapshot.directory.find((t) => t.userId === C.userId);
    check(entry?.mid === section, `C's microphone is on ${entry?.mid} at B, not on the freed ${section}`);
    const sectionsAfter = await mLinesOfOffer(B);
    check(
      sectionsAfter === sectionsBefore,
      `B's description has ${sectionsAfter} sections, it had ${sectionsBefore}`,
    );
    const mic = C.publications.find((p) => p.name === "mic");
    const taken = await checkTakenOver(B, section, statsBefore, mic.streamId);
    return {
      summary: `B's section ${section}: A -> C, ${taken}, ${sectionsAfter} sections; ${summarise(results)}`,
      results,
    };
  },

  /**
   * A publisher's Opus encoder runs on the profile its own client wrote into
   * its offer: A tunes its microphone to 128 kb/s mono and a screen audio
   * track to 160 kb/s stereo, B tunes nothing. The encoder takes its
   * parameters from the media server's descriptions, so they must carry the
   * publisher's own, in the answer and in every later offer.
   */
  async j(s) {
    const A = await s.join("A");
    const screen = await A.publish("screen-audio", "audio", { stereo: true });
    const B = await s.join("B");
    const results = await verifyRoom(s.env, s.clients);

    const sentA = (await A.stats()).outbound;
    const sentB = (await B.stats()).outbound;
    const mic = sentA[A.publications.find((p) => p.name === "mic").mid];
    const stereo = sentA[screen.mid];
    const plain = sentB[B.publications[0].mid];
    const describe = (o) => `${Math.round(o.targetBitrate / 1000)} kb/s (${o.codec})`;
    check(
      mic.codec.includes("maxaveragebitrate=128000") && mic.targetBitrate >= 100_000,
      `A's microphone encodes at ${describe(mic)}`,
    );
    check(
      stereo.codec.includes("stereo=1") && stereo.targetBitrate >= 128_000,
      `A's screen audio encodes at ${describe(stereo)}`,
    );
    check(plain.targetBitrate <= 40_000, `B's untuned microphone encodes at ${describe(plain)}`);
    return {
      summary: `A mic ${describe(mic)}; A screen audio ${describe(stereo)}; B mic ${describe(plain)}`,
      results,
    };
  },

  /** The selected candidate pair of every page ends at the media server. */
  async g(s) {
    await s.join("A");
    await s.join("B");
    await s.join("C");
    const results = await verifyRoom(s.env, s.clients);
    const pairs = results.map(
      (r) =>
        `${r.name}: ${r.pair.local.candidateType} ${r.pair.local.address}:${r.pair.local.port} -> ` +
        `${r.pair.remote.candidateType} ${r.pair.remote.address}:${r.pair.remote.port} ` +
        `(${r.pair.state}, ICE ${r.pair.iceRole}, DTLS ${r.pair.dtlsRole})`,
    );
    return { summary: pairs.join("; "), results };
  },

  /**
   * addTrack instead of addTransceiver: Chromium offers sendrecv, which the
   * media server answers as receive-only, and the track is published. A
   * later addTrack reuses a receiving section the media server created; the
   * media server never takes media there.
   */
  async h(s) {
    const A = await s.join("A", { via: "addTrack" });
    const B = await s.join("B");
    await verifyRoom(s.env, s.clients);

    const offerOfA = (await A.descriptions()).find((d) => d.side === "local" && d.type === "offer");
    const answerToA = (await A.descriptions()).find(
      (d) => d.side === "remote" && d.type === "answer",
    );
    const mid = A.publications[0].mid;
    const directionIn = (sdp) =>
      sdp
        .split("\r\nm=")
        .find((section) => section.includes(`a=mid:${mid}\r\n`))
        ?.match(/a=(sendrecv|sendonly|recvonly|inactive)/)?.[1];
    const transceiver = (await A.snapshot()).transceivers.find((t) => t.mid === mid);
    check(directionIn(answerToA.sdp) === "recvonly", "the media server did not answer recvonly");

    // B already receives A's microphone on a section the media server made.
    const reuse = await B.publish("mic2", "audio", { via: "addTrack" });
    B.publications.pop();
    await settle(s);
    const results = await verifyRoom(s.env, s.clients);
    const snapshotA = await A.snapshot();
    check(
      snapshotA.directory.length === 1,
      `A's directory has ${snapshotA.directory.length} entries after B's addTrack`,
    );
    return {
      summary:
        `A offered ${directionIn(offerOfA.sdp)}, was answered ${directionIn(answerToA.sdp)}, ` +
        `direction ${transceiver.direction}/${transceiver.currentDirection}; ` +
        `B's addTrack reused a server section: ${reuse.reusedTransceiver} (mid ${reuse.mid}, ` +
        `direction ${reuse.direction}); ${summarise(results)}`,
      results,
    };
  },

  /**
   * A call outlives every timer of the media server: the 10 second offer
   * timeout, the 15 seconds after which ICE-lite calls a quiet pair
   * disconnected and the 30 second rules. Media keeps flowing, the media
   * server keeps both sessions connected and closes nothing.
   */
  async l(s) {
    const A = await s.join("A");
    await A.publish("camera", "video");
    await s.join("B");
    await verifyRoom(s.env, s.clients);

    const samples = [];
    for (let i = 0; i < 5; i += 1) {
      await sleep(9000);
      const results = await verifyRoom(s.env, s.clients);
      const stats = await s.env.hub.control.request("GET", "/v1/stats");
      const room = stats.body.rooms.find((r) => r.roomId === s.room);
      const states = room.sessions.map((x) => x.state);
      check(
        states.length === 2 && states.every((x) => x === "connected"),
        `the media server reports ${states}`,
      );
      samples.push(`${(i + 1) * 10}s ${states.join("/")} ${summarise(results)}`);
    }
    return { summary: samples.join(" | ") };
  },

  /**
   * A person with nothing to publish offers one receive-only audio section,
   * which the media server answers inactive, and then receives everybody.
   */
  async m(s) {
    const listener = await s.join("C", { mic: false });
    await listener.receiveOnly("audio");
    await s.join("A");
    const results = await verifyRoom(s.env, s.clients);
    const own = (await listener.snapshot()).transceivers.find((t) => t.mid === "0");
    check(own.currentDirection === "inactive", `C's own section is ${own.currentDirection}`);
    return {
      summary: `C's own section ${own.direction}/${own.currentDirection}; ${summarise(results)}`,
      results,
    };
  },

  /**
   * Through a NAT that loses every 15th camera packet on the way to the
   * media server and on the way from it. The media server asks the
   * publisher to resend what it lost and resends what the receiver lost,
   * each on its own leg, and the picture keeps going. The media server is
   * a second process here, advertising the NAT's address, not its own.
   */
  async n(s) {
    const env = s.env;
    const nat = await LossyNat.listen();
    const logFile = join(env.out, "stellarr-media-nat.log");
    const media = await startMedia({ binary: env.binary, logFile, advertisePort: nat.port });
    nat.target = media.port;
    console.log(
      `  n: second media server pid ${media.pid}, bound 127.0.0.1:${media.port}, ` +
        `advertised ${media.candidate}`,
    );
    const hub = new Hub(media.control);
    await hub.start();
    const inner = new Scenario(
      { ...env, hub, candidate: media.candidate, logText: () => readFileSync(logFile, "utf8") },
      "n",
    );
    try {
      const A = await inner.join("A");
      const camera = await A.publish("camera", "video");
      const B = await inner.join("B");
      await verifyRoom(inner.env, inner.clients);

      const { mapping } = await directoryReady(B, inner.clients);
      const mid = mapping.find((m) => m.publication.kind === "video").mid;
      const before = {
        sent: (await A.stats()).outbound[camera.mid],
        got: (await B.stats()).inbound[mid],
      };
      nat.lose({ pt: 96, every: 15 });
      await sleep(6000);
      const results = await verifyRoom(inner.env, inner.clients);
      nat.lose({ pt: 96, every: 0 });
      const sent = (await A.stats()).outbound[camera.mid];
      const got = (await B.stats()).inbound[mid];

      check(nat.lost.up > 0 && nat.lost.down > 0, `the NAT lost ${JSON.stringify(nat.lost)}`);
      check(sent.codec.startsWith("video/VP8"), `A sends ${sent.codec}, not VP8 on 96`);
      const upNacks = sent.nackCount - before.sent.nackCount;
      const resent = sent.retransmittedPacketsSent - before.sent.retransmittedPacketsSent;
      const downNacks = got.nackCount - before.got.nackCount;
      const repaired = got.retransmittedPacketsReceived - before.got.retransmittedPacketsReceived;
      check(upNacks > 0 && resent > 0, `A got ${upNacks} NACKs and resent ${resent} packets`);
      check(downNacks > 0 && repaired > 0, `B sent ${downNacks} NACKs and got ${repaired} resent`);
      await inner.verifyClean();
      return {
        summary:
          `lost up ${nat.lost.up}, down ${nat.lost.down}; A got ${upNacks} NACKs, resent ${resent}; ` +
          `B sent ${downNacks} NACKs, got ${repaired} resent, lost ${got.packetsLost}, ` +
          `freezes ${got.freezeCount}; ${summarise(results)}`,
        results,
      };
    } finally {
      await inner.dump().catch(() => {});
      await inner.cleanup();
      await hub.stop();
      await media.stop();
      nat.close();
    }
  },

  /**
   * A browser that answers late: the media server's offer is held from the
   * page past the 10 second offer timeout, so the media server rolls it back
   * and offers again before the page has seen the first one. The page then
   * applies both. A late answer must never leave a session alive and broken:
   * it either recovers or ends with `error`, which the client answers by
   * opening a new session. The others are not affected.
   */
  async o(s) {
    await s.join("A");
    const B = await s.join("B");
    const C = await s.join("C");
    await verifyRoom(s.env, s.clients);
    const hub = s.env.hub;
    s.tolerated.add(C.sessionId);

    hub.hold(C.sessionId);
    const held = Date.now();
    await B.publish("camera", "video");
    const offers = () => hub.heldEvents(C.sessionId).filter((e) => e.kind === "offer");
    await waitFor("a second offer to C", async () => offers().length >= 2, 15_000);
    const heldIds = offers().map((e) => e.payload.negotiationId);
    await hub.release(C.sessionId);

    const outcome = await waitFor(
      "C's session to recover or close",
      async () => {
        const closed = hub.eventsOf(C.sessionId).find((e) => e.type === "closed");
        if (closed) return { closed };
        const snapshot = await C.snapshot();
        return snapshot.directory.length === 4 && snapshot.errors.length === 0
          ? { recovered: true }
          : null;
      },
      45_000,
      500,
    );
    const seconds = Math.round((Date.now() - held) / 1000);
    const snapshot = await C.snapshot();
    const answers = hub
      .callsOf(C.sessionId)
      .filter((c) => c.kind === "answer")
      .map((c) => `${c.negotiationId}:${c.status}`);
    const errors = snapshot.errors.map((e) => e.split("\n")[0]);
    let what;
    if (outcome.closed) {
      check(outcome.closed.reason === "error", `C was closed with ${outcome.closed.reason}`);
      what = `closed with ${outcome.closed.reason} ${seconds} s after the hold began`;
      await leave(C);
    } else {
      await verifyReceiver(s.env, C, s.clients);
      what = `recovered ${seconds} s after the hold began`;
    }
    const results = await verifyRoom(s.env, s.clients);
    return {
      summary:
        `held offers ${heldIds}; C answered ${answers}; C's errors ${JSON.stringify(errors)}; ` +
        `${what}; ${summarise(results)}`,
      results,
    };
  },

  /**
   * A receiver that asks only for the microphones, as the web client does
   * while nobody watches a share: a new screen and its sound reach it only
   * once it asks for them, stop when it stops asking and start again with a
   * keyframe, all without renegotiating. The microphones and another viewer
   * carry on throughout.
   */
  async p(s) {
    const A = await s.join("A");
    const B = await s.join("B");
    const C = await s.join("C");
    await verifyRoom(s.env, s.clients);
    const initial = await directoryReady(B, s.clients);
    const microphones = initial.mapping.map((m) => m.mid);
    // From here on B asks for what it wants rather than for every section.
    await B.subscribe(microphones);

    await A.publish("screen", "video", { codec: "VP9", scalabilityMode: "L1T2" });
    await A.publish("screen-audio", "audio", { stereo: true });
    const { mapping, snapshot } = await directoryReady(B, s.clients);
    const screen = mapping.filter((m) => m.publication.name.startsWith("screen"));
    check(screen.length === 2, `B has ${screen.length} screen tracks`);
    const screenVideo = screen.find((m) => m.publication.kind === "video").mid;
    const { mapping: atC } = await directoryReady(C, s.clients);
    await firstMedia(C, atC);
    await sleep(500);
    const blocked = await B.stats();
    for (const { mid } of screen) {
      check((blocked.inbound[mid]?.bytesReceived ?? 0) === 0, `unrequested screen ${mid} reached B`);
    }

    const allMids = mapping.map((m) => m.mid);
    const descriptionsBefore = (await B.descriptions()).length;
    await B.subscribe(allMids);
    await firstMedia(B, mapping);
    const playing = await B.stats();

    await B.subscribe(microphones);
    await sleep(500);
    const paused = await B.stats();
    const otherBefore = await C.stats();
    await sleep(1500);
    const stillPaused = await B.stats();
    const otherAfter = await C.stats();
    for (const { mid } of screen) {
      check(
        (stillPaused.inbound[mid]?.bytesReceived ?? 0) === (paused.inbound[mid]?.bytesReceived ?? 0),
        `paused screen ${mid} still downloads bytes`,
      );
    }
    for (const mid of microphones) {
      check(
        stillPaused.inbound[mid].packetsReceived > paused.inbound[mid].packetsReceived,
        `microphone ${mid} stopped with the screen`,
      );
    }
    const otherVideo = atC.find((m) => m.publication.name === "screen").mid;
    check(
      otherAfter.inbound[otherVideo].framesDecoded > otherBefore.inbound[otherVideo].framesDecoded,
      "pausing B also stopped C's screen",
    );

    await B.subscribe(allMids);
    await waitFor("B to decode a fresh keyframe after resuming", async () => {
      const stats = await B.stats();
      const video = stats.inbound[screenVideo];
      return video.framesDecoded > stillPaused.inbound[screenVideo].framesDecoded &&
        video.keyFramesDecoded > stillPaused.inbound[screenVideo].keyFramesDecoded;
    });
    for (const { mid } of screen) {
      await waitFor(`B's resumed screen ${mid} to advance`, async () =>
        (await B.stats()).inbound[mid].bytesReceived > stillPaused.inbound[mid].bytesReceived,
      );
    }
    // What was skipped while B did not ask is not a loss to B: the numbers
    // carry on where they stopped, so nothing is counted lost or asked for.
    await sleep(1000);
    const resumedStats = await B.stats();
    const repairs = screen.map(({ mid }) => {
      const before = stillPaused.inbound[mid];
      const after = resumedStats.inbound[mid];
      const lost = after.packetsLost - before.packetsLost;
      const nacks = after.nackCount - before.nackCount;
      check(lost <= 5 && nacks <= 5, `B's resumed ${mid} counts ${lost} lost and sent ${nacks} NACKs`);
      return `${mid} lost ${lost}, NACKs ${nacks}`;
    });
    const resumed = await B.snapshot();
    check(JSON.stringify(resumed.directory) === JSON.stringify(snapshot.directory), "subscriptions changed the directory");
    check((await B.descriptions()).length === descriptionsBefore, "subscriptions renegotiated SDP");
    const results = await verifyRoom(s.env, s.clients);
    const video = (stats) => stats.inbound[screenVideo];
    return {
      summary:
        `B's screen blocked until asked for; ${video(playing).framesDecoded} frames, then paused at ` +
        `${video(paused).bytesReceived} bytes for 1.5 s; resumed with keyframes ` +
        `${video(stillPaused).keyFramesDecoded}->${video(resumedStats).keyFramesDecoded} (${repairs.join(", ")}); ` +
        `microphones and C carried on; ${summarise(results)}`,
      results,
    };
  },
  /**
   * One receiver on a poor downlink while the others are on clean links. A
   * reaches the media server through a relay that puts everything sent to it
   * through a rate cap with a 300 ms queue, delay, jitter and losses in
   * bursts; B sends a voice and a camera of 250 kbit/s, C a camera only, so
   * that A's one voice fits even the slowest link. The cap is lowered from
   * 512 to 256 to 128 kbit/s and then lifted. At each cap the allocation is
   * first left to find the link (timed, within 50 s), and once what it holds
   * back has been the expected for 10 s, the next 10 s are measured. A's voice loses no more than the
   * link's own bursts and nothing is dropped for want of room; at 512 B's
   * camera, first in A's list, is decoded and C's is held back; below that
   * both are; once the cap is lifted both come back, within 75 s; and what
   * is held back does not change within a measured window. B and C get
   * everything throughout.
   */
  async q(s) {
    return relayed(s, "q", async (inner, nat) => {
      const { client: A, addresses } = await joinThrough(inner, nat, "A");
      const link = nat.shape(addresses, poorLink(CAPS_KBPS[0]));
      // Each camera goes out only once its publisher has every offer that
      // the others' arrivals bring, so no publication meets one (see e).
      const B = await inner.join("B");
      await directoryReady(B, inner.clients);
      await busyCamera(B, 250_000);
      // C's camera is its first offer, which no offer of the media server
      // can meet.
      const C = await inner.join("C", { mic: false });
      await busyCamera(C, 250_000);
      await directoryReady(C, inner.clients);
      await directoryReady(B, inner.clients);
      const { mapping } = await directoryReady(A, inner.clients);
      const cameraOf = (name) => mapping.find((m) => m.publisher.name === name && m.publication.kind === "video").mid;
      const held = (directory) =>
        directory
          .filter((t) => t.heldBack)
          .map((t) => t.mid)
          .sort()
          .join(",");

      const windows = [];
      const steps = [...CAPS_KBPS, null];
      let returnMs = null;
      const settledMs = [];
      const expectedHeld = (kbps) =>
        kbps === null ? [] : kbps >= 512 ? [cameraOf("C")] : [cameraOf("B"), cameraOf("C")];
      for (const kbps of steps) {
        if (kbps === null) {
          // Lifted: the pictures come back once the estimator has found the
          // room, which may take a fresh look of it after a still estimate.
          nat.unshape(addresses);
          const lifted = Date.now();
          await waitFor(
            "A's cameras to come back on a clean link",
            async () => held((await A.snapshot()).directory) === "",
            75_000,
            250,
          );
          returnMs = Date.now() - lifted;
          console.log(`  q: both cameras back ${returnMs} ms after the cap was lifted`);
          await sleep(3000);
        } else {
          // The allocation first finds the link, from the call's start or
          // from the step before; the window comes once what it holds back
          // has been the expected for the whole settle.
          link.reshape(poorLink(kbps));
          const capped = Date.now();
          const expected = [...expectedHeld(kbps)].sort().join(",");
          let since = null;
          for (;;) {
            const now = Date.now();
            if (held((await A.snapshot()).directory) === expected) since ??= now;
            else since = null;
            if (since !== null && now - since >= SETTLE_MS) break;
            check(now - capped < 60_000, `A's allocation did not settle at ${kbps} kbit/s within 60 s`);
            await sleep(250);
          }
          settledMs.push(`${kbps}: ${since - capped} ms`);
        }
        const before = await A.stats();
        const linkBefore = { ...link.stats };
        const queuedFrom = link.voiceQueueingMs.length;
        const logFrom = (await A.snapshot()).directoryLog.length;
        await sleep(WINDOW_MS);
        const after = await A.stats();
        const measured = await A.measure();
        const snapshot = await A.snapshot();
        const w = receivedBetween(before, after, measured, mapping, WINDOW_MS);
        if (kbps !== null) {
          w.link = linkBetween(linkBefore, link.stats, WINDOW_MS, link.voiceQueueingMs.slice(queuedFrom));
        }
        w.cap = kbps;
        const heldNow = held(snapshot.directory);
        for (const p of w.pictures) p.held = heldNow.split(",").includes(p.mid);
        // Every directory of the window, starting from the one in force.
        const directories = snapshot.directoryLog.slice(Math.max(0, logFrom - 1)).map((d) => held(d.tracks));
        w.changes = directories.filter((d, i) => i > 0 && d !== directories[i - 1]).length;
        windows.push(w);
        const name = kbps === null ? "clean" : `${kbps} kbit/s`;
        w.downlink = await downlinkOf(inner.env.hub, A.sessionId);
        console.log(
          `  q: ${name} ${describeWindow(w)}, held changed ${w.changes} times; ` +
            `media server: ${JSON.stringify(w.downlink)}`,
        );

        if (kbps !== null) {
          check(w.link.kbps <= kbps * 1.02, `the ${kbps} kbit/s link carried ${w.link.kbps} kbit/s`);
          check(w.kbps <= kbps, `A received ${w.kbps} kbit/s on a ${kbps} kbit/s link`);
          check(w.link.queueDrops === 0, `the ${kbps} kbit/s link dropped ${w.link.queueDrops} for want of room`);
          const linkLossPct = (100 * w.link.burstDrops) / Math.max(1, link.stats.offered - linkBefore.offered);
          for (const v of w.voices) {
            check(v.tone, `${name}: A no longer hears ${v.from}'s tone`);
            check(
              v.lostPct <= linkLossPct * 1.5 + 0.5,
              `${name}: ${v.from}'s voice lost ${v.lostPct.toFixed(1)}% where the link lost ${linkLossPct.toFixed(1)}%`,
            );
          }
        }
        check(w.changes === 0, `${name}: what is held back changed ${w.changes} times on a steady link`);
        const expectHeld = expectedHeld(kbps);
        check(
          heldNow === [...expectHeld].sort().join(","),
          `${name}: held back ${heldNow || "nothing"}, not ${expectHeld.join(",") || "nothing"}`,
        );
        for (const p of w.pictures) {
          if (expectHeld.includes(p.mid)) {
            // Padding toward the first held picture goes out on its
            // retransmission stream, so a few bytes of old packets may
            // arrive; nothing of it is decoded.
            check(
              p.fps === 0 && p.keyFrames === 0,
              `${name}: ${p.from}'s held camera still arrives (${p.fps} fps, ${p.keyFrames} keyframes)`,
            );
          } else {
            check(p.fps > 15, `${name}: ${p.from}'s camera decodes at only ${p.fps.toFixed(1)} fps`);
          }
        }
      }

      // The others are on clean links and get everything, whatever A's link.
      const others = [];
      for (const receiver of [B, C]) others.push(await verifyReceiver(inner.env, receiver, inner.clients));
      return {
        summary:
          windows
            .map((w) => `${w.cap === null ? "clean" : `${w.cap} kbit/s`} ${describeWindow(w)}`)
            .join(" | ") +
          `; settled ${settledMs.join(", ")} after each cap; cameras back ${returnMs} ms after the cap was lifted; ` +
          summarise(others),
        windows,
        settledMs,
        returnMs,
      };
    });
  },

  /**
   * A clean link, for what the media server costs a receiver that has room:
   * how soon a newcomer sees a camera of 1.5 Mbit/s that is already running,
   * five times over, and how long a voice spends in the media server while
   * that camera flows, measured at the relay from the RTP timestamps the
   * media server keeps.
   */
  async r(s) {
    return relayed(s, "r", async (inner, nat) => {
      const { client: A, addresses: fromA } = await joinThrough(inner, nat, "A");
      await busyCamera(A, 1_500_000);
      await sleep(4000);

      const firstFrames = [];
      let transit = null;
      const ROUNDS = 5;
      for (let round = 0; round < ROUNDS; round += 1) {
        // Past the one-second window of keyframe requests the last newcomer
        // opened, so each round measures a start, not that window.
        if (round > 0) await sleep(1500);
        const joined = Date.now();
        const { client: B, addresses: toB } = await joinThrough(inner, nat, "B");
        const { mapping } = await directoryReady(B, inner.clients);
        const mid = mapping.find((m) => m.publication.kind === "video").mid;
        await waitFor(
          "B's first decoded frame",
          async () => (await B.stats()).inbound[mid]?.framesDecoded > 0,
          15_000,
          20,
        );
        firstFrames.push(Date.now() - joined);
        if (round === ROUNDS - 1) {
          const tap = voiceTransit(nat, fromA, toB);
          const before = await B.stats();
          await sleep(5000);
          const after = await B.stats();
          transit = tap.stop();
          const w = receivedBetween(before, after, await B.measure(), mapping, 5000);
          transit.received = w;
          check(w.pictures[0].fps > 20, `B decodes A's camera at ${w.pictures[0].fps} fps`);
        } else {
          await leave(B);
        }
      }
      const results = await verifyRoom(inner.env, inner.clients);
      const camera = transit.received.pictures[0];
      return {
        summary:
          `first frame of a 1.5 Mbit/s camera ${firstFrames.join(", ")} ms after joining; ` +
          `voice transit p50 ${transit.p50.toFixed(1)} ms, p95 ${transit.p95.toFixed(1)} ms, ` +
          `max ${transit.max.toFixed(1)} ms over ${transit.n} packets while the camera ran at ` +
          `${Math.round(camera.kbps)} kbit/s ${camera.fps.toFixed(1)} fps; ${summarise(results)}`,
        firstFrames,
        transit,
      };
    });
  },

  // ------------------------------------------------- encrypted calls
  // Run only when named. Every person runs the web client's frame worker on
  // every sender and receiver, keyed from an epoch the script hands out.

  /**
   * What each codec makes of encryption: A publishes a voice and a camera in
   * every video codec the media server forwards, and B, encrypted too, gets
   * the voice, VP8 and VP9 (with temporal layers) in A's colour, while H.264
   * and AV1 never leave A, since they cannot be sent encrypted.
   */
  async xa(s) {
    const A = await s.join("A", { e2ee: {} });
    const wanted = [
      { name: "vp8", codec: "VP8", encrypts: true },
      { name: "vp9", codec: "VP9", encrypts: true },
      { name: "vp9-l1t3", codec: "VP9", scalabilityMode: "L1T3", encrypts: true },
      { name: "h264", codec: "H264", encrypts: false },
      { name: "av1", codec: "AV1", encrypts: false },
    ];
    const skipped = [];
    for (const w of wanted) {
      try {
        await A.publish(w.name, "video", { codec: w.codec, ...(w.scalabilityMode && { scalabilityMode: w.scalabilityMode }) });
      } catch (error) {
        // An engine without the codec or the layers has nothing to show here.
        skipped.push(`${w.name} (${String(error.message ?? error).slice(0, 60)})`);
      }
    }
    const B = await s.join("B", { e2ee: {} });
    const { mapping } = await directoryReady(B, s.clients);
    await sleep(8000);
    const [sent, got, measured, txWorker, rxWorker] = [
      await A.stats(),
      await B.stats(),
      await B.measure(),
      await A.e2eeStats(),
      await B.e2eeStats(),
    ];
    const rows = [];
    for (const publication of A.publications) {
      const m = mapping.find((x) => x.publisher === A && x.publication.name === publication.name);
      const tx = sent.outbound[publication.mid] ?? {};
      const rx = got.inbound[m.mid] ?? {};
      const media = measured[m.mid];
      const sender = transformOf(txWorker, `tx:${publication.name}`);
      const receiver = transformOf(rxWorker, `rx:${m.mid}`);
      const row = {
        track: publication.name,
        codec: tx.codec ?? rx.codec ?? null,
        framesEncoded: tx.framesEncoded ?? 0,
        sealed: sender?.passed ?? 0,
        refusedByCodec: sender?.dropped.codec ?? 0,
        framesDecoded: rx.framesDecoded ?? 0,
        opened: receiver?.passed ?? 0,
        dropped: droppedOf(receiver, "plain", "failed", "unknown-epoch", "unbound"),
        colour: media?.rgb ? nearestColour(media.rgb) : null,
        tone: media?.peakHz ? nearestTone(media.peakHz) : null,
        peakDb: media?.peakDb ?? null,
      };
      rows.push(row);
      const spec = wanted.find((w) => w.name === publication.name);
      if (publication.kind === "audio") {
        check(row.opened > 0 && row.dropped === 0, `B opened ${row.opened} and dropped ${row.dropped} of A's voice`);
        check(row.tone?.name === "A" && row.tone.error < 30 && row.peakDb > -90, `B does not hear A's tone: ${JSON.stringify(row)}`);
      } else if (spec.encrypts) {
        check(row.sealed > 0 && row.opened > 0 && row.framesDecoded > 0, `${row.track} was not sealed, opened and decoded: ${JSON.stringify(row)}`);
        check(row.dropped === 0, `B dropped ${row.dropped} frames of ${row.track}`);
        check(row.colour?.name === "A" && row.colour.distance < 100, `${row.track} decodes to the wrong picture: ${JSON.stringify(row)}`);
      } else {
        check(row.sealed === 0 && row.framesDecoded === 0, `${row.track} left A or was decoded: ${JSON.stringify(row)}`);
      }
    }
    for (const row of rows) console.log(`  ${s.id}: ${JSON.stringify(row)}`);
    return {
      summary:
        `${engine()} via ${B.started.e2eePath}; ` +
        rows
          .map((r) =>
            r.track === "mic"
              ? `mic ${r.codec}: opened ${r.opened}, tone ${r.tone?.name ?? "-"}`
              : `${r.track} ${r.codec ?? "-"}: encoded ${r.framesEncoded}, sealed ${r.sealed}, refused ${r.refusedByCodec}, B decoded ${r.framesDecoded}, colour ${r.colour?.name ?? "-"}`,
          )
          .join("; ") +
        (skipped.length > 0 ? `; not offered here: ${skipped.join(", ")}` : ""),
      rows,
    };
  },

  /**
   * A wrong key: B holds another key under the same epoch. The media server
   * forwards everything, B plays nothing of A, says A's sound and picture are
   * failing and asks for keyframes; the same holds the other way.
   */
  async xb(s) {
    const A = await s.join("A", { e2ee: {} });
    await A.publish("camera", "video");
    const wrong = { ...s.epoch, key: newEpoch(1).key };
    const B = await s.join("B", { e2ee: { epochs: [wrong] } });
    await B.publish("camera", "video");
    await directoryReady(B, s.clients);
    await sleep(6000);
    const rows = [];
    for (const [receiver, sender] of [
      [B, A],
      [A, B],
    ]) {
      const { mapping } = await directoryReady(receiver, s.clients);
      const [stats, measured, worker] = [await receiver.stats(), await receiver.measure(), await receiver.e2eeStats()];
      for (const m of mapping.filter((x) => x.publisher === sender)) {
        const rx = stats.inbound[m.mid] ?? {};
        const t = transformOf(worker, `rx:${m.mid}`);
        const media = measured[m.mid];
        const row = {
          at: receiver.name,
          from: sender.name,
          kind: m.publication.kind,
          packetsReceived: rx.packetsReceived ?? 0,
          framesDecoded: rx.framesDecoded ?? 0,
          opened: t?.passed ?? 0,
          failed: t?.dropped.failed ?? 0,
          keyframeRequests: t?.keyframeRequests ?? 0,
          failing: worker.statuses.filter((x) => x.label === `rx:${m.mid}`).map((x) => x.failing),
          tone: media?.peakHz ? nearestTone(media.peakHz) : null,
          peakDb: media?.peakDb ?? null,
        };
        rows.push(row);
        check(row.packetsReceived > 0, `${row.at} got no packets of ${row.from}'s ${row.kind}; the media server should forward them: ${JSON.stringify(row)}`);
        check(row.opened === 0, `${row.at} opened ${row.from}'s ${row.kind} under a wrong key: ${JSON.stringify(row)}`);
        check(row.failed > 0, `${row.at} saw no frame of ${row.from}'s ${row.kind} fail: ${JSON.stringify(row)}`);
        check(row.failing.includes(true), `${row.at} never said ${row.from}'s ${row.kind} is failing`);
        if (row.kind === "video") {
          check(row.framesDecoded === 0, `${row.at} decoded ${row.framesDecoded} frames of ${row.from}`);
          if (B.started.e2eePath === "script") check(row.keyframeRequests > 0, `${row.at} never asked for a keyframe`);
        } else {
          check(!(row.tone?.name === row.from && row.peakDb > -90), `${row.at} hears ${row.from}'s tone`);
        }
      }
    }
    return {
      summary: rows
        .map((r) => `${r.at}<-${r.from} ${r.kind}: ${r.packetsReceived} packets, ${r.framesDecoded} decoded, ${r.failed} failed, ${r.keyframeRequests} keyframe requests`)
        .join("; "),
      rows,
    };
  },

  /**
   * The epoch changes during a call. In the order a client arranges, both
   * sides hold the new epoch before either sends under it, and nothing is
   * dropped. In the worst order the sender moves first: the receiver drops
   * the frames of the unknown epoch, asks for it at most once a second, and
   * the picture comes back once it has it.
   */
  async xc(s) {
    // The two sides renegotiate after joining, as in scenario e.
    s.allowIgnoredOffers = true;
    const A = await s.join("A", { e2ee: {} });
    await A.publish("camera", "video");
    const B = await s.join("B", { e2ee: {} });
    await B.publish("camera", "video");
    await verifyRoom(s.env, s.clients);
    const { mapping } = await directoryReady(A, s.clients);
    const camB = mapping.find((m) => m.publisher === B && m.publication.kind === "video").mid;
    const micB = mapping.find((m) => m.publisher === B && m.publication.kind === "audio").mid;
    const drops = async () => {
      const worker = await A.e2eeStats();
      return {
        video: droppedOf(transformOf(worker, `rx:${camB}`), "unknown-epoch", "failed"),
        audio: droppedOf(transformOf(worker, `rx:${micB}`), "unknown-epoch", "failed"),
        needs: worker.needs.length,
      };
    };
    let samples = [];
    async function decodedGap(work, ms) {
      // The longest stretch in which B's camera decoded nothing new at A.
      samples = [];
      const end = Date.now() + ms;
      const running = work();
      while (Date.now() < end) {
        samples.push({ at: Date.now(), frames: (await A.stats()).inbound[camB]?.framesDecoded ?? 0 });
        await sleep(100);
      }
      await running;
      let longest = 0;
      let last = samples[0].at;
      for (let i = 1; i < samples.length; i += 1) {
        if (samples[i].frames > samples[i - 1].frames) {
          longest = Math.max(longest, samples[i].at - last);
          last = samples[i].at;
        }
      }
      return Math.max(longest, samples[samples.length - 1].at - last);
    }

    // Arranged: both get epoch 2, then each switches after a grace period.
    const second = newEpoch(2);
    const before = await drops();
    const arrangedGap = await decodedGap(async () => {
      await A.e2eeEpochs([second]);
      await B.e2eeEpochs([second]);
      await sleep(1500);
      await B.e2eeEpochs([], 2);
      await A.e2eeEpochs([], 2);
    }, 5000);
    const arranged = await drops();
    check(
      arranged.video === before.video && arranged.audio === before.audio,
      `the arranged rotation dropped ${arranged.video - before.video} video and ${arranged.audio - before.audio} audio frames`,
    );

    // Worst order: B sends under epoch 3 two seconds before A has it.
    const third = newEpoch(3);
    let learnt = 0;
    const worstGap = await decodedGap(async () => {
      await B.e2eeEpochs([third], 3);
      await sleep(2000);
      learnt = Date.now();
      await A.e2eeEpochs([third]);
    }, 8000);
    const worst = await drops();
    check(worst.video > arranged.video && worst.audio > arranged.audio, "A dropped nothing of an epoch it did not have");
    check(worst.needs - arranged.needs >= 1 && worst.needs - arranged.needs <= 3, `A asked for the epoch ${worst.needs - arranged.needs} times in 2 s`);
    // The first sample after A learnt the epoch in which B's picture moved on.
    const back = samples.find((x, i) => i > 0 && x.at >= learnt && x.frames > samples[i - 1].frames);
    check(back !== undefined, "B's picture never came back after A learnt the epoch");
    const recovered = back.at;
    const results = await verifyRoom(s.env, s.clients);
    return {
      summary:
        `both first: 0 dropped, longest picture gap ${arrangedGap} ms; ` +
        `sender first: A dropped ${worst.video - arranged.video} video and ${worst.audio - arranged.audio} audio frames, ` +
        `asked for the epoch ${worst.needs - arranged.needs} times, longest picture gap ${worstGap} ms, ` +
        `picture back within ${recovered - learnt} ms of A learning the epoch; ${summarise(results)}`,
      arrangedGap,
      worstGap,
    };
  },

  /**
   * A browser without encoded transforms in an encrypted call: it publishes
   * nothing, and nothing of it reaches anybody.
   */
  async xd(s) {
    const A = await s.join("A", { e2ee: {} });
    const strip = () => {
      delete window.RTCRtpScriptTransform;
      delete RTCRtpSender.prototype.createEncodedStreams;
      delete RTCRtpReceiver.prototype.createEncodedStreams;
    };
    const C = await s.join("C", { mic: false, e2ee: {}, initScript: strip });
    let refusal = null;
    try {
      await C.publish("mic", "audio");
    } catch (error) {
      refusal = String(error.message ?? error);
    }
    check(C.started.refused !== null, `C did not notice it cannot encrypt: ${JSON.stringify(C.started)}`);
    check(refusal?.includes("refused"), "C published in an encrypted call without encrypting");
    await sleep(2000);
    const offers = s.env.hub.calls.filter((c) => c.sessionId === C.sessionId && c.kind === "offer");
    const snapshot = await A.snapshot();
    check(snapshot.directory.every((t) => t.userId !== C.userId), "A's directory lists a track of C");
    s.tolerated.add(C.sessionId);
    return { summary: `C refused (${C.started.refused}); offers from C: ${offers.length}; A has nothing of C` };
  },

  /**
   * What encryption costs a browser: a voice and a busy camera (640x480, up
   * to 1.2 Mbit/s) each way, without a transform, through a worker that
   * passes frames on untouched, through one that copies them once, and
   * encrypted. The modes are interleaved, twice each, so a machine busy with
   * something else hurts them alike; the CPU is the whole browser's, from ps.
   */
  async xe(s) {
    check(engine() === "chromium", "the CPU is read from Chromium's processes only");
    const modes = [
      ["none", null],
      ["touch", { worker: "touch" }],
      ["copy", { worker: "copy" }],
      ["encrypted", {}],
    ];
    const runs = [];
    for (const [name, e2ee] of [...modes, ...modes]) {
      s.room = randomUUID();
      const A = await s.join("A", { e2ee });
      await busyCamera(A, 1_200_000);
      const B = await s.join("B", { e2ee });
      await busyCamera(B, 1_200_000);
      await verifyRoom(s.env, s.clients);
      await sleep(6000);
      const ids = s.env.identities;
      const windowMs = 20_000;
      const load = loadAverage();
      const cpu0 = [browserCpuSeconds(ids[0]), browserCpuSeconds(ids[1])];
      const st0 = await A.stats();
      await sleep(windowMs);
      const cpu1 = [browserCpuSeconds(ids[0]), browserCpuSeconds(ids[1])];
      const st1 = await A.stats();
      const pct = (i) => (cpu0[i] === null ? null : (100 * (cpu1[i] - cpu0[i])) / (windowMs / 1000));
      const video = (st) => Object.values(st.inbound).find((x) => x.kind === "video" && x.packetsReceived > 0);
      const camera = (st) => Object.values(st.outbound).find((x) => x.kind === "video");
      const row = {
        mode: name,
        cpuA: pct(0),
        cpuB: pct(1),
        load,
        cameraKbps: Math.round(((camera(st1).bytesSent - camera(st0).bytesSent) * 8) / windowMs),
        receivedFps: (video(st1).framesDecoded - video(st0).framesDecoded) / (windowMs / 1000),
        downlinkA: await downlinkOf(s.env.hub, A.sessionId),
      };
      runs.push(row);
      console.log(`  xe: ${JSON.stringify(row)}`);
      for (const c of [...s.clients]) await leave(c);
      s.clients = [];
    }
    const mean = (mode) => {
      const rows = runs.filter((r) => r.mode === mode);
      const values = rows.flatMap((r) => [r.cpuA, r.cpuB]).filter((v) => v !== null);
      return values.reduce((a, b) => a + b, 0) / values.length;
    };
    // The frame format alone, in the page, by frame size.
    const A = await s.join("A", { e2ee: null, mic: false });
    const bench = await A.page.evaluate(async () => {
      const f = await import("/sframe.js");
      const base = await crypto.subtle.importKey("raw", crypto.getRandomValues(new Uint8Array(32)), "HKDF", false, ["deriveBits"]);
      const kid = f.keyIdOf(1, crypto.getRandomValues(new Uint8Array(6)));
      const context = { epochId: crypto.randomUUID(), conversationId: crypto.randomUUID(), senderId: crypto.randomUUID() };
      const sealing = await f.deriveFrameKey(base, context, kid, "encrypt");
      const opening = await f.deriveFrameKey(base, context, kid, "decrypt");
      const rows = [];
      for (const size of [160, 1200, 6000, 30000]) {
        const frame = crypto.getRandomValues(new Uint8Array(size));
        const sealed = [];
        const n = 500;
        let started = performance.now();
        for (let i = 0; i < n; i += 1) sealed.push(await f.sealFrame(sealing, kid, i, frame, 0));
        const seal = ((performance.now() - started) * 1000) / n;
        started = performance.now();
        for (let i = 0; i < n; i += 1) await f.openFrame(opening, f.parseFrame(sealed[i]), sealed[i]);
        const open = ((performance.now() - started) * 1000) / n;
        rows.push({ size, sealUs: Number(seal.toFixed(1)), openUs: Number(open.toFixed(1)) });
      }
      return rows;
    });
    return {
      summary:
        ["none", "touch", "copy", "encrypted"].map((m) => `${m} ${mean(m).toFixed(1)}%`).join(", ") +
        ` of one core per browser (mean of 2 runs x 2 browsers); per frame ` +
        bench.map((r) => `${r.size} B seal ${r.sealUs} us open ${r.openUs} us`).join(", "),
      runs,
      bench,
    };
  },

  /**
   * What a variable bitrate gives away: the same speech-like voice sent with
   * and without a constant bitrate, encrypted, read as the sizes of the frames
   * the sender's worker sealed and the bitrate on the wire.
   */
  async xf(s) {
    const rows = [];
    for (const cbr of [false, true]) {
      s.room = randomUUID();
      const A = await s.join("A", { e2ee: {}, cbr, pattern: "speech" });
      const B = await s.join("B", { e2ee: {}, mic: false });
      await B.receiveOnly("audio");
      await directoryReady(B, s.clients);
      await sleep(3000);
      const w0 = transformOf(await A.e2eeStats(), "tx:mic");
      const st0 = await A.stats();
      await sleep(10_000);
      const w1 = transformOf(await A.e2eeStats(), "tx:mic");
      const st1 = await A.stats();
      const frames = w1.frames - w0.frames;
      const bytes = w1.bytesIn - w0.bytesIn;
      const squares = w1.sizeSquares - w0.sizeSquares;
      const meanSize = bytes / frames;
      const mic = (st) => Object.values(st.outbound).find((x) => x.kind === "audio");
      const row = {
        cbr,
        frames,
        meanBytes: Number(meanSize.toFixed(1)),
        sdBytes: Number(Math.sqrt(Math.max(0, squares / frames - meanSize * meanSize)).toFixed(1)),
        minBytes: w1.sizeMin,
        maxBytes: w1.sizeMax,
        kbps: Math.round(((mic(st1).bytesSent - mic(st0).bytesSent) * 8) / 10_000),
      };
      rows.push(row);
      console.log(`  xf: ${JSON.stringify(row)}`);
      for (const c of [...s.clients]) await leave(c);
      s.clients = [];
    }
    const [vbr, cbr] = rows;
    check(cbr.sdBytes < vbr.sdBytes, `the constant bitrate varies more than the variable one: ${JSON.stringify(rows)}`);
    return {
      summary: rows
        .map((r) => `${r.cbr ? "CBR" : "VBR"}: ${r.kbps} kbit/s, frames ${r.meanBytes} ± ${r.sdBytes} B (${r.minBytes}..${r.maxBytes})`)
        .join("; "),
      rows,
    };
  },

  /**
   * A camera turned off and on again in an encrypted private call: the
   * receiver's section is the same, its receiver keeps the transform it has,
   * and every new picture is opened and decoded in the publisher's colour.
   */
  async xg(s) {
    const A = await s.join("A", { e2ee: {} });
    const B = await s.join("B", { e2ee: {} });
    await directoryReady(A, s.clients);
    const sections = [];
    let opened = 0;
    for (const how of ["remove", "stop", null]) {
      await A.publish("camera", "video");
      await verifyRoom(s.env, s.clients);
      const { mapping } = await directoryReady(B, s.clients);
      const mid = mapping.find((m) => m.publication.kind === "video").mid;
      sections.push(mid);
      const stats = await B.e2eeStats();
      const transforms = stats.transforms.filter((t) => t.label === `rx:${mid}`);
      check(transforms.length === 1, `B has ${transforms.length} transforms on ${mid}`);
      const worker = transformOf(stats, `rx:${mid}`);
      check(worker.passed > opened, `B's receiver on ${mid} opened no new frame: ${JSON.stringify(worker)}`);
      opened = worker.passed;
      if (!how) break;
      await A.unpublish("camera", how);
      await sectionFreed(B, mid);
    }
    check(sections.every((mid) => mid === sections[0]), `the camera moved between sections ${sections}`);
    const results = await verifyRoom(s.env, s.clients);
    return { summary: `B's section ${sections[0]} carried the camera ${sections.length} times; ${summarise(results)}`, results };
  },
};

async function settle(s) {
  await Promise.all(s.clients.map((c) => s.env.hub.posted(c.sessionId)));
  await sleep(500);
}

// -------------------------------------------------------------------- main

// An interrupted run still stops what it started: the exit handler of
// lib/media.mjs ends the media servers, and Playwright closes its browsers.
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    console.log(`stopped by ${signal}`);
    process.exit(130);
  });
}

async function main() {
  const wanted = process.argv.slice(2);
  const e2ee = e2eeFromEnv();
  // The encrypted-call scenarios run only when named. Run encrypted, scenario i
  // has nothing to prove: H.264 and AV1 cannot be sent encrypted (scenario xa).
  const ids = Object.keys(SCENARIOS)
    .sort()
    .filter((id) =>
      wanted.length > 0 ? wanted.includes(id) : !id.startsWith("x") && !(e2ee && id === "i"),
    );
  const out =
    process.env.STELLARR_SMOKE_OUT ?? mkdtempSync(join(tmpdir(), "stellarr-media-smoke-"));
  mkdirSync(out, { recursive: true });

  const binary =
    process.env.STELLARR_MEDIA_BINARY ?? resolve(here, "../target/release/stellarr-media");
  const logFile = join(out, "stellarr-media.log");
  const executablePath = browserPath();
  console.log(`browser ${engine()} ${executablePath ?? "(Playwright's own)"}\noutput ${out}`);

  const cleanup = [];
  let failed = 0;
  try {
    const media = await startMedia({
      binary,
      logFile,
      advertiseHost: process.env.STELLARR_SMOKE_ADVERTISE ?? "127.0.0.1",
    });
    cleanup.push(() => media.stop());
    const hub = new Hub(media.control);
    await hub.start();
    cleanup.push(() => hub.stop());

    const page = await servePage(join(here, "page"));
    cleanup.push(() => page.close());

    const identities = writeFixtures(out);
    const browsers = {};
    for (const identity of identities) {
      browsers[identity.name] = await launch(executablePath, identity);
      cleanup.push(() => browsers[identity.name].close());
    }

    const env = {
      binary,
      hub,
      url: page.url,
      candidate: media.candidate,
      identities,
      browsers,
      out,
      logText: () => readFileSync(logFile, "utf8"),
      e2ee,
    };
    console.log(`media server pid ${media.pid}, candidate ${media.candidate}, encrypted ${e2ee ? JSON.stringify(e2ee) : "no"}`);

    const report = {};
    for (const id of ids) {
      const scenario = new Scenario(env, id);
      const started = Date.now();
      try {
        const result = await SCENARIOS[id](scenario);
        await scenario.verifyClean();
        report[id] = { passed: true, ...result };
        console.log(`PASS ${id} (${Date.now() - started} ms) ${result.summary}`);
      } catch (error) {
        failed += 1;
        report[id] = { passed: false, error: String(error.stack ?? error) };
        console.log(`FAIL ${id} (${Date.now() - started} ms) ${error.message}`);
        if (!(error instanceof Failure)) console.log(error.stack);
      }
      await scenario.dump().catch((error) => console.log(`dump of ${id} failed: ${error}`));
      await scenario.cleanup();
    }

    if (hub.resets > 0 || hub.pumpError) {
      failed += 1;
      console.log(`FAIL hub: ${hub.resets} resets, ${hub.pumpError ?? "no poll error"}`);
    }
    writeFileSync(join(out, "results.json"), JSON.stringify(report, null, 2));
  } finally {
    for (const step of cleanup.reverse()) {
      try {
        const result = await step();
        if (result && typeof result === "object" && "code" in result) {
          console.log(`media server stopped with ${JSON.stringify(result)}`);
        }
      } catch (error) {
        console.log(`cleanup: ${error}`);
      }
    }
  }

  console.log(failed === 0 ? "all scenarios passed" : `${failed} failed`);
  process.exitCode = failed === 0 ? 0 : 1;
}

await main();
