import { connectPort, createMedia } from "./media.js";

const pollInterval = 200;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const transient = error => !error.status || error.status >= 500;
const clone = value => structuredClone(value);

function embeddedTransport() {
  const channel = new MessageChannel();
  const pending = new Map();
  let serial = 0;
  channel.port1.onmessage = ({ data }) => {
    const call = pending.get(data?.id);
    if (!call) return;
    if (data.error) call.reject(Object.assign(new Error(data.error), { status: data.status }));
    else call.resolve(data.value);
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
  try { opened = await request({ op: "open", initialState }); }
  catch (error) { transport.close?.(); throw error; }
  const launchMode = opened.launchMode === "personal" ? "personal" : "voice";
  const context = { standalone: opened.standalone ?? false, me: opened.me, sessionId: opened.sessionId,
    launchMode, place: launchMode === "personal" ? null : opened.place ?? null,
    locale: opened.locale ?? navigator.language };
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
  let sending = false;
  let deciding = false;
  let sequence = 0;
  let writes = Promise.resolve();
  let media;

  function apply(result) {
    if (closed) return;
    if (result.version >= version) {
      version = result.version;
      epoch = result.epoch;
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
      try { apply(await retry({ ...input, ...decision })); }
      catch (error) {
        if ([400, 413].includes(error.status)) apply(await retry({ ...input, error: error.message.slice(0, 512) }));
      }
    } finally { deciding = false; }
  }

  async function poll() {
    const began = performance.now();
    try {
      const packet = await request({ op: "read", after: revision });
      if (closed) return;
      clockOffset = packet.now - (began + performance.now()) / 2;
      connection.update("connected");
      apply(packet);
      for (const message of packet.messages ?? []) {
        const value = message.binary ? Uint8Array.from(atob(message.data), char => char.charCodeAt(0)) : message.data;
        for (const listener of subscriptions.get(message.kind) ?? []) notify(listener, clone(value), { from: message.from });
      }
      void processCommand(packet).catch(() => {});
    } catch (error) {
      if (closed) return;
      if ([401, 403, 404, 410].includes(error.status)) { finish(); transport.close?.(); }
      else connection.update("reconnecting");
    } finally {
      if (!closed) timer = setTimeout(poll, pollInterval);
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

  async function flushMessages() {
    if (sending) return;
    sending = true;
    try {
      while (!closed && outbox.length) {
        const packet = outbox.shift();
        try { await request(packet); } catch { /* Live messages are best effort. */ }
      }
    } finally { sending = false; }
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
    for (;;) {
      const reply = await retry(input);
      if (reply.error !== undefined) throw new Error(reply.error);
      if (!reply.pending) {
        // Make the confirmed revision visible when the caller resumes after submit().
        if (revision < reply.revision) {
          const packet = await retry({ op: "sync", after: revision });
          apply(packet);
        }
        return reply.result;
      }
      await delay(pollInterval);
    }
  }

  const session = {
    context, limits, capabilities: ["context", "state", "theme", "participants", "messages", "clock", "seats",
      ...(hosted ? ["host", "commands", "assignments"] : []), ...permissions],
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
    media = createMedia({ context, permissions, volume: opened.volume, youtube: opened.youtube, request, send, on, participants: participants.api });
    session.media = media;
  } catch (error) { await depart("close").catch(() => {}); throw error; }
  timer = setTimeout(poll, pollInterval);
  return session;
}
