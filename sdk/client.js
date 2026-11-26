import { connectPort, createMedia } from "./media.js";

const pollInterval = 200;
// Room presence expires after five seconds without a call.
const heartbeatInterval = 2000;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const transient = error => !error.status || error.status >= 500;
const clone = value => structuredClone(value);

function embeddedTransport() {
  const channel = new MessageChannel();
  const pending = new Map();
  let serial = 0;
  channel.port1.onmessage = ({ data }) => {
    if (data?.update) { send.onUpdate?.(data.update); return; }
    const call = pending.get(data?.id);
    if (!call) return;
    if (typeof data.error === "string") {
      const message = data.error || `Activity request failed (HTTP ${data.status}).`;
      call.reject(Object.assign(new Error(message), { status: data.status }));
    } else call.resolve(data.value);
  };
  connectPort("stellarr:dev-connect:v1", channel.port2);
  const send = input => new Promise((resolve, reject) => {
    const id = ++serial;
    const finish = fn => value => {
      clearTimeout(timer);
      pending.delete(id);
      fn(value);
    };
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error("The Stellarr session did not respond."));
    }, 12000);
    pending.set(id, { resolve: finish(resolve), reject: finish(reject) });
    channel.port1.postMessage({ id, input });
  });
  send.close = () => {
    channel.port1.close();
    for (const call of pending.values()) call.reject(new Error("The Stellarr session closed."));
  };
  return send;
}

function notify(listener, ...values) {
  try { listener(...values); }
  catch (error) { globalThis.reportError?.(error); }
}

function observable(initial) {
  let current = initial;
  const listeners = new Set();
  return {
    api: {
      get: () => clone(current),
      subscribe(listener) {
        listeners.add(listener);
        notify(listener, clone(current));
        return () => listeners.delete(listener);
      },
    },
    update(value) {
      if (JSON.stringify(current) === JSON.stringify(value)) return;
      current = value;
      for (const listener of listeners) notify(listener, clone(value));
    },
  };
}

/** Classify the primary pointer and the physical screen, even inside a narrow frame. */
export function detectDevice() {
  if (!window.matchMedia?.("(pointer: coarse)").matches) return "desktop";
  return Math.min(window.screen.width, window.screen.height) >= 600 ? "tablet" : "phone";
}

function deviceTheme(dark) {
  return { mode: dark ? "dark" : "light", colors: dark
    ? { background: "#17191d", surface: "#24272d", text: "#f2f3f5", muted: "#b1b7c2", accent: "#90b8ff", border: "#474c55" }
    : { background: "#ffffff", surface: "#f4f5f7", text: "#20242a", muted: "#596371", accent: "#285baf", border: "#c8ced6" } };
}

function encodeMessage(data) {
  if (data instanceof ArrayBuffer || ArrayBuffer.isView(data)) {
    const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    let parts = "";
    for (let i = 0; i < bytes.length; i += 8192) parts += String.fromCharCode(...bytes.subarray(i, i + 8192));
    return { data: btoa(parts), binary: true, bytes: bytes.byteLength };
  }
  const json = JSON.stringify(data);
  if (json === undefined) throw new Error("Expected JSON data or bytes.");
  return { data: JSON.parse(json), binary: false, bytes: new TextEncoder().encode(json).length };
}

/** Join the activity running in the surrounding Stellarr page or local development room. */
export async function connect({ initialState = {} } = {}) {
  if (window.parent === window) throw new Error("Open this activity through Stellarr or run stellarr dev.");
  const transport = embeddedTransport();
  const frame = crypto.randomUUID();
  const request = input => transport({ ...input, frame });
  const started = performance.now();
  let opened;
  try { opened = await request({ op: "open", initialState, roomEvents: true }); }
  catch (error) { transport.close?.(); throw error; }
  const launchMode = opened.launchMode === "personal" ? "personal" : "voice";
  const context = { standalone: opened.standalone ?? false, me: opened.me, sessionId: opened.sessionId,
    launchMode, place: launchMode === "personal" ? null : opened.place ?? null,
    locale: opened.locale ?? navigator.language, device: opened.device ?? detectDevice() };
  const locale = observable(context.locale);
  const device = observable(context.device);
  const { limits, permissions = [] } = opened;
  const hosted = opened.authority === "host";
  const me = context.me.id;
  let clockOffset = opened.now - (started + performance.now()) / 2;
  let { state, revision } = opened.changes[0];
  let { version, epoch } = opened;
  const participants = observable(opened.participants);
  const seats = observable(opened.seats);
  const host = observable(opened.host);
  const assignment = observable(opened.assignment);
  const connection = observable("connected");
  const scheme = window.matchMedia?.("(prefers-color-scheme: dark)");
  let explicitTheme = opened.theme;
  const theme = observable(explicitTheme ?? deviceTheme(scheme?.matches));
  const onScheme = () => { if (!explicitTheme) theme.update(deviceTheme(scheme?.matches)); };
  scheme?.addEventListener("change", onScheme);
  const stateListeners = new Set();
  const subscriptions = new Map();
  const handlers = new Map();
  const outbox = [];
  let timer;
  let closed = false;
  let reading = false;
  let immediateRead = false;
  let readAgain = false;
  const settlements = new Set();
  let sendsInFlight = 0;
  let deciding = false;
  let sequence = 0;
  let writes = Promise.resolve();
  let media;

  function apply(result) {
    if (closed) return;
    if (result.version >= version && Array.isArray(result.participants)) {
      version = result.version;
      epoch = result.epoch;
      context.me = result.participants.find(person => person.id === me) ?? context.me;
      participants.update(result.participants);
      seats.update(result.seats);
      assignment.update(result.assignment);
      host.update(result.host);
    }
    if (result.theme) { explicitTheme = result.theme; theme.update(result.theme); }
    for (const item of result.changes ?? []) {
      if (item.revision <= revision) continue;
      state = item.state;
      revision = item.revision;
      for (const listener of stateListeners) notify(listener, clone(state), revision);
    }
  }

  function finish() {
    if (closed) return;
    closed = true;
    clearTimeout(timer);
    for (const notify of settlements) notify();
    window.removeEventListener("pagehide", onHide);
    scheme?.removeEventListener("change", onScheme);
    media?.dispose?.();
    connection.update("closed");
    outbox.length = 0;
  }

  function depart(op) {
    if (closed) return Promise.resolve();
    const reply = request({ op });
    finish();
    return reply.then(() => {}).finally(() => transport.close?.());
  }
  function onHide(event) { if (!event.persisted) void depart("disconnect").catch(() => {}); }
  window.addEventListener("pagehide", onHide);
  function assertOpen() { if (closed) throw new Error("The Stellarr session closed."); }

  async function retry(input) {
    let backoff = pollInterval;
    for (;;) {
      assertOpen();
      try { return await request(input); }
      catch (error) {
        if (!transient(error) || closed) throw error;
        await delay(backoff);
        backoff = Math.min(2000, 2 * backoff);
      }
    }
  }

  async function processCommand(packet) {
    if (deciding || !packet.command || packet.commandRevision !== revision || packet.epoch !== epoch) return;
    deciding = true;
    let answered = false;
    const { from, id, kind, data } = packet.command;
    const input = { op: "decide", from, id, epoch, version: packet.version, expected: revision };
    try {
      let decision;
      try {
        const handler = handlers.get(kind);
        if (!handler) throw new Error("Unknown command: " + kind);
        const value = await handler(clone(data), { from, id }, clone(state), { assignments: clone(packet.checkpoint) });
        decision = { patch: value?.patch, result: value?.result, assignments: value?.assignments };
      } catch (error) { decision = { error: String(error?.message ?? error).slice(0, 512) }; }
      try { apply(await retry({ ...input, ...decision })); answered = true; }
      catch (error) {
        if ([400, 413].includes(error.status)) {
          apply(await retry({ ...input, error: error.message.slice(0, 512) }));
          answered = true;
        }
      }
    } finally {
      deciding = false;
      if (answered && opened.roomEvents) wakeRead();
    }
  }

  function deliver(message) {
    const value = message.binary ? Uint8Array.from(atob(message.data), char => char.charCodeAt(0)) : message.data;
    for (const listener of subscriptions.get(message.kind) ?? []) notify(listener, clone(value), { from: message.from });
  }

  function wakeRead() {
    if (closed) return;
    if (reading) { readAgain = true; return; }
    if (immediateRead) return;
    immediateRead = true;
    clearTimeout(timer);
    timer = setTimeout(poll, 0);
  }
  transport.onUpdate = update => {
    if (closed || !opened.roomEvents) return;
    if (update.message) { deliver(update.message); return; }
    if (update.settled) for (const notify of settlements) notify(update.settled);
    wakeRead();
  };

  async function poll() {
    reading = true;
    immediateRead = false;
    const began = performance.now();
    try {
      const packet = await request({ op: "read", after: revision, version, roomEvents: !!opened.roomEvents });
      if (closed) return;
      clockOffset = packet.now - (began + performance.now()) / 2;
      connection.update("connected");
      context.locale = packet.locale ?? opened.locale ?? navigator.language;
      context.device = packet.device ?? detectDevice();
      locale.update(context.locale);
      device.update(context.device);
      apply(packet);
      for (const message of packet.messages ?? []) deliver(message);
      void processCommand(packet).catch(() => {});
    } catch (error) {
      if (closed) return;
      if ([401, 403, 404, 410].includes(error.status)) { finish(); transport.close?.(); }
      else connection.update("reconnecting");
    } finally {
      reading = false;
      const interval = opened.roomEvents ? heartbeatInterval : pollInterval;
      immediateRead = readAgain;
      if (!closed) timer = setTimeout(poll, readAgain ? 0 : interval);
      readAgain = false;
    }
  }

  async function commit(change, writeEpoch) {
    const writeId = ++sequence;
    for (;;) {
      assertOpen();
      const after = revision;
      const functional = typeof change === "function";
      const patch = functional ? change(clone(state)) : change;
      if (patch === undefined && functional) return;
      const result = await retry({ op: "write", after, patch, sequence: writeId,
        expected: functional ? after : undefined, epoch: hosted ? writeEpoch : undefined });
      apply(result);
      if (!result.conflict) return result.revision;
    }
  }

  function flushMessages() {
    // Hub calls run in wire order. Keep a small window so RTT does not limit
    // the send rate to a single message per network round trip.
    const windowSize = opened.roomEvents ? 4 : 1;
    while (!closed && outbox.length && sendsInFlight < windowSize) {
      const packet = outbox.shift();
      sendsInFlight++;
      void request(packet).catch(() => {}).finally(() => {
        sendsInFlight--;
        flushMessages();
      });
    }
  }

  function send(kind, data, { to, latest = false } = {}) {
    if (closed || connection.api.get() !== "connected" || typeof kind !== "string" || !kind || kind.length > 128 ||
        (to !== undefined && (typeof to !== "string" || !to || to.length > 128))) return false;
    let payload;
    try { payload = encodeMessage(data); } catch { return false; }
    if (payload.bytes > limits.messageBytes) return false;
    const packet = { op: "send", kind, data: payload.data, binary: payload.binary, latest, to,
      epoch: hosted && host.api.get() === me ? epoch : undefined };
    const index = latest ? outbox.findIndex(item => item.latest && item.kind === kind && item.to === to) : -1;
    if (index >= 0) outbox[index] = packet;
    else {
      if (outbox.length >= 32) return false;
      outbox.push(packet);
    }
    void flushMessages();
    return true;
  }

  function on(kind, listener) {
    const listeners = subscriptions.get(kind) ?? new Set();
    subscriptions.set(kind, listeners);
    listeners.add(listener);
    return () => { listeners.delete(listener); if (!listeners.size) subscriptions.delete(kind); };
  }

  async function updateSeat(claim) {
    assertOpen();
    apply(await request({ op: "seat", claim }));
  }
  async function submit(kind, data, { id = crypto.randomUUID() } = {}) {
    const input = { op: "submit", kind, data: clone(data), id };
    let wake;
    let settled = false;
    function onSettlement(commandId) {
      if (commandId === undefined || commandId === id) { settled = true; wake?.(); }
    }
    if (opened.roomEvents) settlements.add(onSettlement);
    try {
      for (;;) {
        settled = false;
        const reply = await retry(input);
        if (reply.error !== undefined) throw new Error(reply.error);
        if (!reply.pending) {
          if (revision < reply.revision) apply(await retry({ op: "sync", after: revision }));
          return reply.result;
        }
        if (opened.roomEvents && !settled) {
          // The result survives a dropped notification or a socket reconnect.
          await new Promise(resolve => {
            const timeout = setTimeout(resolve, heartbeatInterval);
            wake = () => { clearTimeout(timeout); resolve(); };
          });
          wake = undefined;
        } else if (!opened.roomEvents) await delay(pollInterval);
      }
    } finally { settlements.delete(onSettlement); }
  }

  const session = {
    context, limits, capabilities: ["context", "state", "theme", "participants", "messages", "clock", "seats",
      ...(opened.progress ? ["progress"] : []), ...(hosted ? ["host", "commands", "assignments"] : []), ...permissions],
    locale: locale.api, device: device.api,
    progress: {
      load() { return retry({ op: "loadProgress" }); },
      async save(data, expectedRevision) {
        // Capture the exact JSON once. A serialization error must reach the caller before network retries.
        const snapshot = JSON.parse(JSON.stringify(data));
        const reply = await retry({ op: "saveProgress", data: snapshot, expected: expectedRevision });
        return reply.revision;
      },
    },
    now: () => performance.now() + clockOffset, send, on,
    close: () => depart("close"), connection: connection.api, theme: theme.api, participants: participants.api,
    seats: { ...seats.api, claim: () => updateSeat(true), release: () => updateSeat(false) },
    assignments: { ...assignment.api, checkpoint() { assertOpen(); return request({ op: "checkpoint", epoch }); } },
    host: { ...host.api, async transfer(to) { assertOpen(); apply(await request({ op: "transfer", to, epoch })); } },
    commands: {
      handle(kind, handler) {
        handlers.set(kind, handler);
        return () => { if (handlers.get(kind) === handler) handlers.delete(kind); };
      }, submit,
    },
    state: {
      get: () => clone(state),
      subscribe(listener) {
        stateListeners.add(listener);
        notify(listener, clone(state), revision);
        return () => stateListeners.delete(listener);
      },
      set(change) {
        const writeEpoch = epoch;
        const captured = typeof change === "function" ? change : clone(change);
        const result = writes.then(() => commit(captured, writeEpoch));
        writes = result.catch(() => {});
        return result;
      },
    },
  };
  try {
    media = createMedia({ context, permissions, volume: opened.volume, embedHosts: opened.embedHosts, hostedEmbeds: opened.hostedEmbeds });
    session.media = media;
  } catch (error) { await depart("close").catch(() => {}); throw error; }
  timer = setTimeout(poll, opened.roomEvents ? 0 : pollInterval);
  return session;
}
