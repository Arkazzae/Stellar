// The smallest client of Stellarr Media that keeps the "Negotiation rules" of
// docs/stellarr-media.md: one RTCPeerConnection with max-bundle, the browser
// offers what it publishes and answers what the media server offers, it is
// the impolite side of a collision, every arriving track is filed under the
// person the directory names, and it asks for every section the directory
// names, since the media server forwards nothing it is not asked for.
//
// The Node script plays the hub. It calls the functions on `window.smoke` and
// hands every event of this page's session to `smoke.signal`; the page sends
// its own signals through `window.hubSend`, which Node exposes.

"use strict";

/** The Opus parameters the web client writes into its local descriptions. */
const OPUS_TUNING = {
  useinbandfec: "1",
  usedtx: "0",
  maxaveragebitrate: "128000",
  stereo: "0",
  "sprop-stereo": "0",
  minptime: "10",
};

/** The screen audio profile of docs/rtc-protocol.md: 160 kb/s in stereo. */
const SCREEN_AUDIO_TUNING = {
  ...OPUS_TUNING,
  maxaveragebitrate: "160000",
  stereo: "1",
  "sprop-stereo": "1",
};

const state = {
  pc: null,
  sessionId: null,
  sent: 0,
  tuneOpus: false,
  // Streams whose audio is tuned as screen audio rather than as a microphone.
  stereoStreams: new Set(),
  // Every negotiation step, incoming signal and outgoing offer runs on this
  // one chain. Whether to ignore an offer is decided when it is processed, so
  // an offer that follows an answer is never judged by a state the answer is
  // about to change.
  chain: Promise.resolve(),
  directory: [],
  directoryLog: [],
  // Asks for every section of each directory until a scenario says what it
  // wants instead. The last list sent, so an unchanged one is not sent again.
  subscribeToAll: true,
  subscribed: "[]",
  offersIgnored: [],
  offersAnswered: [],
  answersApplied: 0,
  negotiationNeeded: 0,
  arrivals: [],
  published: {},
  descriptions: [],
  errors: [],
  closed: null,
  audio: null,
  timers: [],
  cbr: false,
  userId: null,
  // The person's tone and colour, made in the page where there are no fake devices.
  synthetic: null,
  // The frame worker of an encrypted page, and why a page that must encrypt cannot.
  e2ee: null,
  refused: null,
};

// ------------------------------------------------------------ encryption
//
// An encrypted page runs the web client's own frame worker (served from
// web/src/stellarr/lib/voice/e2ee) on every sender and receiver, as an
// encrypted call does, keyed from epochs the script hands it.

function fromBase64(text) {
  return Uint8Array.from(atob(text), (c) => c.charCodeAt(0));
}

/** "script" for RTCRtpScriptTransform, "streams" for Chromium's createEncodedStreams, or null. */
function transformPath(wanted) {
  const script = typeof RTCRtpScriptTransform === "function";
  const streams = typeof RTCRtpSender !== "undefined" && "createEncodedStreams" in RTCRtpSender.prototype;
  if (wanted === "streams") return streams ? "streams" : null;
  if (script) return "script";
  return streams ? "streams" : null;
}

async function workerEpochs(epochs) {
  return Promise.all(
    epochs.map(async ({ sequence, epochId, key }) => ({
      sequence,
      epochId,
      base: await crypto.subtle.importKey("raw", fromBase64(key), "HKDF", false, ["deriveBits"]),
    })),
  );
}

async function startE2ee(options) {
  const path = transformPath(options.path);
  if (path === null) {
    // An encrypted call never goes out in the clear: this page publishes nothing.
    state.refused = "no encoded transforms";
    return;
  }
  // "frame" is the app's worker; "touch" and "copy" are the harness's baselines.
  const kind = options.worker ?? "frame";
  const worker =
    kind === "frame"
      ? new Worker("/frame-worker.js", { type: "module" })
      : new Worker("/copy-worker.js", { name: kind });
  const e2ee = { worker, kind, path, pending: new Map(), next: 0, statuses: [], needs: [], receivers: [] };
  worker.onmessage = ({ data }) => {
    if (data.type === "stats") {
      e2ee.pending.get(data.id)?.(data);
      e2ee.pending.delete(data.id);
    } else if (data.type === "status") {
      e2ee.statuses.push({ at: performance.now(), label: data.label, failing: data.failing });
    } else if (data.type === "need-epoch") {
      e2ee.needs.push({ at: performance.now(), sequence: data.sequence });
    }
  };
  worker.onerror = (event) => state.errors.push(`frame worker: ${event.message}`);
  worker.postMessage({ type: "init", conversationId: options.conversationId, self: state.userId });
  worker.postMessage({ type: "epochs", epochs: await workerEpochs(options.epochs) });
  worker.postMessage({ type: "send", sequence: options.send });
  state.e2ee = e2ee;
}

function attachTransform(target, options) {
  const e2ee = state.e2ee;
  if (!e2ee) return;
  if (e2ee.path === "script") {
    target.transform = new RTCRtpScriptTransform(e2ee.worker, options);
    return;
  }
  const { readable, writable } = target.createEncodedStreams();
  e2ee.worker.postMessage({ type: "streams", readable, writable, options }, [readable, writable]);
}

/**
 * Receivers wait for the directory to name whose frames they open, and follow
 * it when the media server hands their section to another person's track.
 */
function bindReceivers() {
  const e2ee = state.e2ee;
  if (!e2ee) return;
  for (const receiver of e2ee.receivers) {
    const entry = state.directory.find((t) => t.mid === receiver.mid);
    if (!entry || entry.userId === receiver.publisher) continue;
    receiver.publisher = entry.userId;
    e2ee.worker.postMessage({ type: "bind", label: receiver.label, sender: entry.userId });
  }
}

/** Tells the worker what each sender negotiated, for a browser whose frames do not say. */
function codecHints() {
  const e2ee = state.e2ee;
  if (!e2ee) return;
  for (const [name, published] of Object.entries(state.published)) {
    const mimeType = published.transceiver.sender.getParameters().codecs?.[0]?.mimeType;
    if (mimeType) e2ee.worker.postMessage({ type: "codec", label: `tx:${name}`, mimeType });
  }
}

async function e2eeStats() {
  const e2ee = state.e2ee;
  if (!e2ee) return { refused: state.refused };
  const id = ++e2ee.next;
  const reply = await new Promise((resolve) => {
    e2ee.pending.set(id, resolve);
    e2ee.worker.postMessage({ type: "stats", id });
  });
  return { path: e2ee.path, kind: e2ee.kind, transforms: reply.transforms, statuses: e2ee.statuses, needs: e2ee.needs };
}

async function e2eeEpochs(epochs, send) {
  const e2ee = state.e2ee;
  e2ee.worker.postMessage({ type: "epochs", epochs: await workerEpochs(epochs) });
  if (send !== undefined && send !== null) e2ee.worker.postMessage({ type: "send", sequence: send });
}

function run(task) {
  const result = state.chain.then(task);
  state.chain = result.catch((error) => {
    state.errors.push(String((error && error.stack) || error));
  });
  return result;
}

function send(kind, payload) {
  // The number lets the hub check that signals reach it in the order they
  // were sent.
  state.sent += 1;
  window.hubSend(state.sessionId, state.sent, kind, payload);
}

function record(side, type, sdp, extra = {}) {
  state.descriptions.push({ at: performance.now(), side, type, sdp, ...extra });
}

function setDirectory(source, tracks) {
  state.directory = tracks;
  state.directoryLog.push({ at: performance.now(), source, tracks });
  bindReceivers();
  if (state.subscribeToAll) {
    // The order of the list is the order of importance: the voices first,
    // then the pictures, each in the order the directory has them.
    const ordered = [
      ...tracks.filter((t) => t.kind === "audio"),
      ...tracks.filter((t) => t.kind !== "audio"),
    ];
    requestMids(ordered.map((t) => t.mid));
  }
}

/** Sends the complete list of sections to forward, in order, unless it is the one sent last. */
function requestMids(mids) {
  const list = JSON.stringify([...mids]);
  if (list === state.subscribed) return;
  state.subscribed = list;
  send("subscriptions", { mids: JSON.parse(list) });
}

/** From now on the page asks for exactly `mids`, whatever the directory says. */
function subscribe(mids) {
  state.subscribeToAll = false;
  requestMids(mids);
}

/**
 * What `tuneLocalDescription` of the web client does to an audio section:
 * Opus first in the payload list, the Opus fmtp parameters merged in and a
 * 20 ms packet time.
 */
function tuneSection(section) {
  if (!section.startsWith("m=audio")) return section;
  const trailing = section.endsWith("\r\n");
  const lines = section.replace(/\r\n$/, "").split("\r\n");
  const opus = new Set();
  for (const line of lines) {
    const match = /^a=rtpmap:(\d+) opus\//i.exec(line);
    if (match) opus.add(match[1]);
  }
  if (opus.size === 0) return section;

  const msid = lines.find((line) => line.startsWith("a=msid:"));
  const stream = msid ? msid.slice("a=msid:".length).split(" ")[0] : null;
  const base = state.stereoStreams.has(stream) ? SCREEN_AUDIO_TUNING : OPUS_TUNING;
  // An encrypted call's voice may ask for a constant bitrate.
  const tuning = state.cbr ? { ...base, cbr: "1" } : base;

  const [media, port, proto, ...pts] = lines[0].split(" ");
  const ordered = [...pts.filter((p) => opus.has(p)), ...pts.filter((p) => !opus.has(p))];
  lines[0] = [media, port, proto, ...ordered].join(" ");

  let lastFmtp = -1;
  for (let i = 0; i < lines.length; i += 1) {
    const fmtp = /^a=fmtp:(\d+) (.*)$/.exec(lines[i]);
    if (!fmtp || !opus.has(fmtp[1])) continue;
    const merged = new Map();
    for (const part of fmtp[2].split(";")) {
      const [key, value = ""] = part.trim().split("=");
      if (key) merged.set(key, value);
    }
    for (const [key, value] of Object.entries(tuning)) merged.set(key, value);
    const params = [...merged].map(([k, v]) => (v === "" ? k : `${k}=${v}`)).join(";");
    lines[i] = `a=fmtp:${fmtp[1]} ${params}`;
    lastFmtp = i;
  }

  const ptime = lines.findIndex((line) => line.startsWith("a=ptime:"));
  if (ptime !== -1) lines[ptime] = "a=ptime:20";
  else lines.splice(lastFmtp === -1 ? lines.length : lastFmtp + 1, 0, "a=ptime:20");

  return lines.join("\r\n") + (trailing ? "\r\n" : "");
}

function tuneOpus(sdp) {
  if (!state.tuneOpus) return sdp;
  const [session, ...sections] = sdp.split(/\r\n(?=m=)/);
  return [session, ...sections.map(tuneSection)].join("\r\n");
}

function stable() {
  const pc = state.pc;
  if (pc.signalingState === "stable" || pc.signalingState === "closed") return Promise.resolve();
  return new Promise((resolve) => {
    const check = () => {
      if (pc.signalingState === "stable" || pc.signalingState === "closed") {
        pc.removeEventListener("signalingstatechange", check);
        resolve();
      }
    };
    pc.addEventListener("signalingstatechange", check);
  });
}

/** Offers what this page publishes, as soon as no offer of its own is open. */
async function negotiate() {
  for (;;) {
    await stable();
    const sent = await run(async () => {
      if (state.pc.signalingState !== "stable") return false;
      const offer = await state.pc.createOffer();
      await state.pc.setLocalDescription({ type: "offer", sdp: tuneOpus(offer.sdp) });
      const sdp = state.pc.localDescription.sdp;
      record("local", "offer", sdp);
      send("offer", { type: "offer", sdp });
      return true;
    });
    if (sent) return;
  }
}

async function handle(event) {
  // A signal or a closure that names another session is dropped.
  if (event.sessionId !== state.sessionId) return;
  if (event.type === "closed") {
    state.closed = event.reason;
    return;
  }

  const pc = state.pc;
  const payload = event.payload;
  switch (event.kind) {
    case "answer": {
      if (pc.signalingState !== "have-local-offer") {
        throw new Error(`an answer arrived in ${pc.signalingState}`);
      }
      record("remote", "answer", payload.sdp);
      await pc.setRemoteDescription({ type: "answer", sdp: payload.sdp });
      state.answersApplied += 1;
      codecHints();
      return;
    }
    case "offer": {
      // Impolite: an offer of the media server that meets one of ours being
      // made or waiting for its answer is ignored. The media server rolls
      // its own back and offers again after answering ours.
      if (pc.signalingState !== "stable") {
        state.offersIgnored.push(payload.negotiationId);
        record("remote", "offer-ignored", payload.sdp, { negotiationId: payload.negotiationId });
        return;
      }
      // The directory comes with the offer, so it is in hand before any
      // track of the offer arrives, and its sections are asked for before
      // the answer that makes them live. The media server forgets what it was
      // asked for on a section it hands to another track, so the list goes
      // out again with every offer, the same as the last or not.
      if (state.subscribed !== "[]") state.subscribed = null;
      setDirectory("offer", payload.tracks);
      record("remote", "offer", payload.sdp, { negotiationId: payload.negotiationId });
      await pc.setRemoteDescription({ type: "offer", sdp: payload.sdp });
      const answer = await pc.createAnswer();
      await pc.setLocalDescription({ type: "answer", sdp: tuneOpus(answer.sdp) });
      const sdp = pc.localDescription.sdp;
      record("local", "answer", sdp, { negotiationId: payload.negotiationId });
      send("answer", { type: "answer", sdp, negotiationId: payload.negotiationId });
      state.offersAnswered.push(payload.negotiationId);
      return;
    }
    case "tracks":
      setDirectory("tracks", payload.tracks);
      return;
    default:
      throw new Error(`unknown signal ${event.kind}`);
  }
}

function audioContext() {
  if (!state.audio) state.audio = new AudioContext();
  state.audio.resume().catch(() => {});
  return state.audio;
}

/** Plays a remote track muted, so it is decoded, and taps audio for its tone. */
function attach(arrival) {
  const stream = new MediaStream([arrival.track]);
  const element = document.createElement(arrival.kind === "video" ? "video" : "audio");
  element.muted = true;
  element.autoplay = true;
  element.playsInline = true;
  element.srcObject = stream;
  document.body.append(element);
  element.play().catch(() => {});
  arrival.element = element;

  if (arrival.kind === "audio") {
    const ctx = audioContext();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 8192;
    analyser.smoothingTimeConstant = 0.3;
    ctx.createMediaStreamSource(stream).connect(analyser);
    arrival.analyser = analyser;
  }
}

function onTrack(event) {
  const mid = event.transceiver.mid;
  const entry = state.directory.find((t) => t.mid === mid);
  // Every receiver gets one transform for as long as the connection lives,
  // bound to the publisher the directory names; one the directory does not
  // name yet drops what it gets. A section that starts receiving again for
  // another track reports its receiver again, which keeps the transform it has.
  if (state.e2ee && !state.e2ee.receivers.some((r) => r.receiver === event.receiver)) {
    const label = `rx:${mid}`;
    attachTransform(event.receiver, {
      role: "receive",
      kind: event.track.kind,
      label,
      ...(entry ? { sender: entry.userId } : {}),
    });
    state.e2ee.receivers.push({
      mid,
      label,
      receiver: event.receiver,
      publisher: entry ? entry.userId : null,
    });
  }
  const arrival = {
    at: performance.now(),
    mid,
    kind: event.track.kind,
    trackId: event.track.id,
    streams: event.streams.map((s) => s.id),
    signalingState: state.pc.signalingState,
    directoryKnown: Boolean(entry),
    directoryUserId: entry ? entry.userId : null,
    mutedAtArrival: event.track.muted,
    events: [],
    track: event.track,
  };
  for (const name of ["mute", "unmute", "ended"]) {
    event.track.addEventListener(name, () => {
      arrival.events.push({ name, at: performance.now() });
    });
  }
  attach(arrival);
  state.arrivals.push(arrival);
}

async function start({ sessionId, tuneOpus: tune = false, cbr = false, userId = null, e2ee = null, synthetic = null }) {
  state.sessionId = sessionId;
  state.tuneOpus = tune;
  state.cbr = cbr;
  state.userId = userId;
  state.synthetic = synthetic;
  if (e2ee) await startE2ee(e2ee);
  const config = { iceServers: [], bundlePolicy: "max-bundle" };
  if (state.e2ee?.path === "streams") config.encodedInsertableStreams = true;
  const pc = new RTCPeerConnection(config);
  state.pc = pc;
  pc.addEventListener("track", onTrack);
  pc.addEventListener("icecandidate", (event) => {
    send("candidate", event.candidate ? event.candidate.toJSON() : null);
  });
  pc.addEventListener("negotiationneeded", () => {
    state.negotiationNeeded += 1;
  });
  return {
    isSecureContext: window.isSecureContext,
    e2eePath: state.e2ee ? state.e2ee.path : null,
    refused: state.refused,
    userAgent: navigator.userAgent,
  };
}

/** The person's tone or colour made in the page, for an engine without fake devices. */
function syntheticCapture(kind) {
  const { toneHz, rgb } = state.synthetic;
  if (kind === "audio") {
    const ctx = audioContext();
    const oscillator = ctx.createOscillator();
    oscillator.frequency.value = toneHz;
    const gain = ctx.createGain();
    gain.gain.value = 0.3;
    const destination = ctx.createMediaStreamDestination();
    oscillator.connect(gain).connect(destination);
    oscillator.start();
    return destination.stream;
  }
  const canvas = document.createElement("canvas");
  canvas.width = 320;
  canvas.height = 240;
  const g = canvas.getContext("2d");
  let x = 0;
  // A small moving square keeps the encoder making frames, as the fake camera does.
  const draw = () => {
    g.fillStyle = `rgb(${rgb[0]},${rgb[1]},${rgb[2]})`;
    g.fillRect(0, 0, 320, 240);
    g.fillStyle = `rgb(${Math.min(255, rgb[0] + 60)},${Math.min(255, rgb[1] + 60)},${Math.min(255, rgb[2] + 60)})`;
    g.fillRect(x, 108, 24, 24);
    x = (x + 4) % 296;
  };
  draw();
  const stream = canvas.captureStream(30);
  state.timers.push(setInterval(draw, 1000 / 30));
  return stream;
}

/**
 * Something like speech for an encoder: a voice-band tone with noise that
 * comes and goes in bursts of a few hundred milliseconds, so a variable
 * bitrate shows in the packet sizes.
 */
function speechLike() {
  const ctx = audioContext();
  const tone = ctx.createOscillator();
  tone.frequency.value = 220;
  tone.type = "sawtooth";
  const noise = ctx.createBufferSource();
  const buffer = ctx.createBuffer(1, ctx.sampleRate * 2, ctx.sampleRate);
  const samples = buffer.getChannelData(0);
  for (let i = 0; i < samples.length; i += 1) samples[i] = Math.random() * 2 - 1;
  noise.buffer = buffer;
  noise.loop = true;
  const noiseLevel = ctx.createGain();
  noiseLevel.gain.value = 0.2;
  const gate = ctx.createGain();
  gate.gain.value = 0;
  const destination = ctx.createMediaStreamDestination();
  tone.connect(gate);
  noise.connect(noiseLevel).connect(gate);
  gate.connect(destination);
  tone.start();
  noise.start();
  const pattern = [400, 150, 250, 500, 300, 200];
  let step = 0;
  state.timers.push(
    setInterval(() => {
      step += 1;
      gate.gain.setTargetAtTime(step % 2 === 0 ? 0.4 : 0, ctx.currentTime, 0.01);
      tone.frequency.setValueAtTime(180 + (step % 5) * 40, ctx.currentTime);
    }, pattern[step % pattern.length]),
  );
  return destination.stream;
}

async function capture(kind, busy, pattern) {
  if (busy) return busyCamera(busy);
  if (kind === "audio" && pattern === "speech") return speechLike();
  if (state.synthetic) return syntheticCapture(kind);
  const constraints =
    kind === "audio"
      ? { audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false } }
      : { video: { width: 320, height: 240, frameRate: 30 } };
  return navigator.mediaDevices.getUserMedia(constraints);
}

/**
 * A camera that costs what a real one does: blocks of noise around the
 * person's colour, new every frame, which an encoder cannot squeeze. The
 * noise averages out, so the colour a receiver measures is still the
 * person's.
 */
function busyCamera({ rgb, width = 640, height = 480, fps = 30 }) {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  const block = 8;
  const spread = 60;
  const draw = () => {
    for (let y = 0; y < height; y += block) {
      for (let x = 0; x < width; x += block) {
        const c = rgb.map((v) =>
          Math.max(0, Math.min(255, Math.round(v + (Math.random() - 0.5) * spread))),
        );
        ctx.fillStyle = `rgb(${c[0]},${c[1]},${c[2]})`;
        ctx.fillRect(x, y, block, block);
      }
    }
  };
  draw();
  const stream = canvas.captureStream(fps);
  const timer = setInterval(draw, 1000 / fps);
  stream.getVideoTracks()[0].addEventListener("ended", () => clearInterval(timer));
  state.timers.push(timer);
  return stream;
}

/**
 * Publishes the fake microphone or camera. `via` is "transceiver" for
 * addTransceiver(track, { direction: "sendonly", streams }) or "addTrack";
 * `codec` is put first in the codec preferences, `scalabilityMode` is asked
 * of the encoder, `maxBitrate` caps it, `stereo` tunes an audio track as
 * screen audio, and `busy` makes the camera one that costs a real bitrate.
 */
async function publish(
  name,
  kind,
  { via = "transceiver", codec, scalabilityMode, maxBitrate, stereo, busy, pattern } = {},
) {
  if (state.refused) throw new Error(`refused to publish: ${state.refused}`);
  const stream = await capture(kind, busy, pattern);
  if (stereo) state.stereoStreams.add(stream.id);
  const track = stream.getTracks()[0];
  let transceiver;
  if (via === "addTrack") {
    const sender = state.pc.addTrack(track, stream);
    transceiver = state.pc.getTransceivers().find((t) => t.sender === sender);
  } else {
    const init = { direction: "sendonly", streams: [stream] };
    if (scalabilityMode || maxBitrate) {
      init.sendEncodings = [
        { ...(scalabilityMode && { scalabilityMode }), ...(maxBitrate && { maxBitrate }) },
      ];
    }
    transceiver = state.pc.addTransceiver(track, init);
  }
  // From the sender's first frame on, before anything is negotiated.
  attachTransform(transceiver.sender, { role: "send", kind, label: `tx:${name}` });
  if (state.e2ee && !codec) {
    // As the web client does: an encrypted call offers only what can be
    // encrypted. WebKit would otherwise send H.264, which never leaves.
    const allowed = kind === "audio" ? ["audio/opus"] : ["video/vp8", "video/vp9", "video/rtx", "video/red", "video/ulpfec"];
    const all = RTCRtpReceiver.getCapabilities(kind).codecs;
    transceiver.setCodecPreferences(all.filter((c) => allowed.includes(c.mimeType.toLowerCase())));
  }
  if (codec) {
    // The codec is moved to the front and nothing is removed, as the screen
    // share of the web client does.
    const all = RTCRtpReceiver.getCapabilities(kind).codecs;
    const wanted = (c) => c.mimeType.toLowerCase() === `${kind}/${codec}`.toLowerCase();
    transceiver.setCodecPreferences([...all.filter(wanted), ...all.filter((c) => !wanted(c))]);
  }
  state.published[name] = { transceiver, stream, track, kind };
  const reused = transceiver.mid !== null;
  await negotiate();
  return {
    streamId: stream.id,
    trackId: track.id,
    mid: transceiver.mid,
    reusedTransceiver: reused,
    direction: transceiver.direction,
  };
}

/**
 * Stops publishing: "remove" is removeTrack, which leaves the section
 * inactive, and "stop" is transceiver.stop(), which rejects it and lets the
 * browser recycle its m-line later.
 */
async function unpublish(name, how) {
  const published = state.published[name];
  if (how === "stop") published.transceiver.stop();
  else state.pc.removeTrack(published.transceiver.sender);
  published.track.stop();
  delete state.published[name];
  await negotiate();
  return { direction: published.transceiver.direction, mid: published.transceiver.mid };
}

/** Replaces a published camera with a new capture in a new stream. */
async function swap(name) {
  const published = state.published[name];
  const stream = await capture("video");
  const track = stream.getVideoTracks()[0];
  const sender = published.transceiver.sender;
  await sender.replaceTrack(track);
  sender.setStreams(stream);
  published.track.stop();
  published.track = track;
  published.stream = stream;
  await negotiate();
  return { streamId: stream.id, trackId: track.id, mid: published.transceiver.mid };
}

function snapshot() {
  const pc = state.pc;
  return {
    signalingState: pc.signalingState,
    connectionState: pc.connectionState,
    iceConnectionState: pc.iceConnectionState,
    directory: state.directory,
    directoryLog: state.directoryLog,
    transceivers: pc.getTransceivers().map((t) => ({
      mid: t.mid,
      direction: t.direction,
      currentDirection: t.currentDirection,
      kind: t.receiver.track.kind,
      receiverTrackId: t.receiver.track.id,
      receiverMuted: t.receiver.track.muted,
      receiverReadyState: t.receiver.track.readyState,
      sending: t.sender.track !== null,
    })),
    arrivals: state.arrivals.map(({ track, element, analyser, ...rest }) => ({
      ...rest,
      muted: track.muted,
      readyState: track.readyState,
    })),
    offersIgnored: state.offersIgnored,
    offersAnswered: state.offersAnswered,
    answersApplied: state.answersApplied,
    negotiationNeeded: state.negotiationNeeded,
    errors: state.errors,
    closed: state.closed,
  };
}

async function stats() {
  const report = await state.pc.getStats();
  const codecs = {};
  report.forEach((s) => {
    if (s.type === "codec") codecs[s.id] = `${s.mimeType} ${s.sdpFmtpLine || ""}`.trim();
  });

  const inbound = {};
  const outbound = {};
  let pair = null;
  report.forEach((s) => {
    if (s.type === "inbound-rtp") {
      inbound[s.mid] = {
        kind: s.kind,
        ssrc: s.ssrc,
        codec: codecs[s.codecId] || null,
        packetsReceived: s.packetsReceived || 0,
        bytesReceived: s.bytesReceived || 0,
        packetsLost: s.packetsLost || 0,
        framesDecoded: s.framesDecoded || 0,
        keyFramesDecoded: s.keyFramesDecoded || 0,
        frameWidth: s.frameWidth || 0,
        nackCount: s.nackCount || 0,
        pliCount: s.pliCount || 0,
        firCount: s.firCount || 0,
        retransmittedPacketsReceived: s.retransmittedPacketsReceived || 0,
        freezeCount: s.freezeCount || 0,
        totalFreezesDuration: s.totalFreezesDuration || 0,
        framesReceived: s.framesReceived || 0,
        framesDropped: s.framesDropped || 0,
        concealedSamples: s.concealedSamples || 0,
        silentConcealedSamples: s.silentConcealedSamples || 0,
        concealmentEvents: s.concealmentEvents || 0,
        totalSamplesReceived: s.totalSamplesReceived || 0,
        jitterBufferDelay: s.jitterBufferDelay || 0,
        jitterBufferEmittedCount: s.jitterBufferEmittedCount || 0,
      };
    }
    if (s.type === "outbound-rtp") {
      outbound[s.mid] = {
        kind: s.kind,
        ssrc: s.ssrc,
        codec: codecs[s.codecId] || null,
        packetsSent: s.packetsSent || 0,
        bytesSent: s.bytesSent || 0,
        framesEncoded: s.framesEncoded || 0,
        keyFramesEncoded: s.keyFramesEncoded || 0,
        pliCount: s.pliCount || 0,
        firCount: s.firCount || 0,
        nackCount: s.nackCount || 0,
        retransmittedPacketsSent: s.retransmittedPacketsSent || 0,
        targetBitrate: s.targetBitrate || 0,
        scalabilityMode: s.scalabilityMode || null,
      };
    }
    if (s.type === "transport" && s.selectedCandidatePairId) {
      const selected = report.get(s.selectedCandidatePairId);
      const local = selected && report.get(selected.localCandidateId);
      const remote = selected && report.get(selected.remoteCandidateId);
      const describe = (c) =>
        c && {
          address: c.address,
          port: c.port,
          protocol: c.protocol,
          candidateType: c.candidateType,
        };
      pair = selected && {
        state: selected.state,
        nominated: selected.nominated,
        local: describe(local),
        remote: describe(remote),
        availableOutgoingBitrate: selected.availableOutgoingBitrate || 0,
        dtlsState: s.dtlsState,
        dtlsRole: s.dtlsRole,
        iceRole: s.iceRole,
      };
    }
  });
  return { inbound, outbound, pair };
}

/**
 * The audio level each receiving audio section last carried, by mid. Chromium
 * fills it only from the ssrc-audio-level header extension, so a number here
 * proves the extension was forwarded.
 */
function levels() {
  const out = {};
  for (const transceiver of state.pc.getTransceivers()) {
    if (transceiver.receiver.track.kind !== "audio" || !transceiver.mid) continue;
    const sources = transceiver.receiver.getSynchronizationSources();
    out[transceiver.mid] = sources.map((source) => source.audioLevel ?? null);
  }
  return out;
}

/** Offers to receive only, as a client with nothing to publish must. */
async function receiveOnly(kind) {
  state.pc.addTransceiver(kind, { direction: "recvonly" });
  await negotiate();
}

/** The tone of every audio track and the colour of every video track, by mid. */
function measure() {
  const out = {};
  for (const arrival of state.arrivals) {
    if (arrival.kind === "audio") {
      const analyser = arrival.analyser;
      const bins = new Float32Array(analyser.frequencyBinCount);
      analyser.getFloatFrequencyData(bins);
      let peak = 1;
      for (let i = 1; i < bins.length; i += 1) if (bins[i] > bins[peak]) peak = i;
      out[arrival.mid] = {
        kind: "audio",
        peakHz: (peak * state.audio.sampleRate) / analyser.fftSize,
        peakDb: bins[peak],
      };
    } else {
      const element = arrival.element;
      if (!element.videoWidth) {
        out[arrival.mid] = { kind: "video", rgb: null };
        continue;
      }
      const canvas = document.createElement("canvas");
      canvas.width = 16;
      canvas.height = 16;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      ctx.drawImage(element, 0, 0, 16, 16);
      const data = ctx.getImageData(0, 0, 16, 16).data;
      const sum = [0, 0, 0];
      for (let i = 0; i < data.length; i += 4) {
        sum[0] += data[i];
        sum[1] += data[i + 1];
        sum[2] += data[i + 2];
      }
      const pixels = data.length / 4;
      out[arrival.mid] = {
        kind: "video",
        rgb: sum.map((v) => Math.round(v / pixels)),
        width: element.videoWidth,
      };
    }
  }
  return out;
}

function close() {
  for (const timer of state.timers) clearInterval(timer);
  for (const published of Object.values(state.published)) published.track.stop();
  if (state.pc) state.pc.close();
  if (state.e2ee) state.e2ee.worker.terminate();
}

window.smoke = {
  start,
  publish,
  unpublish,
  swap,
  signal: (event) => {
    run(() => handle(event));
  },
  receiveOnly,
  subscribe,
  snapshot,
  stats,
  levels,
  measure,
  descriptions: () => state.descriptions,
  close,
  e2eeStats,
  e2eeEpochs,
};
