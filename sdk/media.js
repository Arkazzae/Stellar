/** The parent owns device access and camera pixels. This side only handles SDK events and layout. */
// The instance serves this module at a reserved URL. In a same-host
// sandbox the parent gives each launch a separate handshake nonce.
export function connectPort(type, port) {
  const nonce = globalThis.origin === "null" && /^#stellarr=([0-9a-f-]{36})$/i.exec(location.hash)?.[1];
  if (nonce) window.parent.postMessage({ type, nonce }, new URL(import.meta.url).origin, [port]);
  else window.parent.postMessage(type, "*", [port]);
}

export function createMedia({ context, permissions = [], volume: initialVolume = 1, youtube = false }) {
  const channel = new MessageChannel();
  const port = channel.port1;
  const pending = new Map();
  const devices = new Map();
  const mounts = new Set();
  const listeners = new Set();
  let volume = initialVolume ?? 1;
  let player;
  let closed = false;
  let sequence = 0;
  const post = data => { if (!closed) port.postMessage(data); };

  port.onmessage = ({ data }) => {
    if (!data) return;
    if (data.op === "volume" && Number.isFinite(data.value)) {
      volume = Math.max(0, Math.min(1, data.value));
      for (const fn of listeners) fn(volume);
    } else if (data.op === "youtubeEvent" && player?.id === data.id) {
      player.update(data);
    } else if (data.op === "level") {
      devices.get("microphoneLevel")?.update(data.value);
    } else if (data.op === "ended") {
      devices.get(data.kind)?.end(data.reason);
    } else if (data.op === "reply") {
      pending.get(data.id)?.(data.error);
    }
  };
  if (window.parent !== window)
    connectPort("stellarr:media:v1", channel.port2);

  async function open(kind) {
    if (closed || window.parent === window || !permissions.includes("media." + kind))
      throw new Error("This activity cannot use " + kind + ".");
    if (devices.has(kind)) return devices.get(kind).handle;
    const id = ++sequence;
    await new Promise((resolve, reject) => {
      const finish = error => {
        pending.delete(id);
        clearTimeout(timer);
        if (error) reject(new Error(error));
        else resolve();
      };
      const timer = setTimeout(() => { post({ op: "cancel", id }); finish("Device request timed out."); }, 60000);
      pending.set(id, finish);
      post({ op: "open", kind, id });
    });
    if (closed) throw new Error("Session closed.");
    if (devices.has(kind)) return devices.get(kind).handle;
    const levels = new Set(), endings = new Set();
    const end = reason => {
      if (!devices.delete(kind)) return;
      for (const fn of endings) fn(reason);
      levels.clear();
      endings.clear();
    };
    const handle = {
      stop() { if (devices.get(kind)?.handle === handle) { post({ op: "stop", kind }); end("stopped"); } },
      onEnded(fn) { endings.add(fn); return () => endings.delete(fn); },
    };
    if (kind === "microphoneLevel") handle.subscribe = fn => { levels.add(fn); return () => levels.delete(fn); };
    devices.set(kind, { handle, end, update(value) { for (const fn of levels) fn(value); } });
    return handle;
  }
  function trackBounds(element, report) {
    let previous, raf;
    function position() {
      const r = element.getBoundingClientRect();
      const bounds = [r.left, r.top, r.width, r.height, innerWidth, innerHeight];
      const key = bounds.join(",") + element.isConnected;
      if (key !== previous) {
        report(element.isConnected ? bounds : [0, 0, 0, 0, innerWidth, innerHeight]);
        previous = key;
      }
      raf = requestAnimationFrame(position);
    }
    position();
    return () => cancelAnimationFrame(raf);
  }
  function mount(participant, element) {
    if (closed || !permissions.includes("media.cameraTile") || !(element instanceof Element)) return () => {};
    const id = ++sequence;
    const untrack = trackBounds(element, bounds => post({ op: "mount", id, participant, bounds }));
    const remove = () => { untrack(); post({ op: "unmount", id }); mounts.delete(remove); };
    mounts.add(remove);
    return remove;
  }
  function mountYouTube(videoId, element, events) {
    if (closed || !(element instanceof Element) || !/^[a-zA-Z0-9_-]{11}$/.test(videoId))
      throw new Error("Invalid YouTube player.");
    player?.destroy();
    const id = ++sequence;
    let time = 0, duration = 0, state = -1;
    post({ op: "youtubeOpen", id, videoId });
    const untrack = trackBounds(element, bounds => post({ op: "youtubeLayout", id, bounds }));
    const command = (command, value) => post({ op: "youtubeCommand", id, command, value });
    const timeout = setTimeout(() => events.error(0), 16000);
    function destroy() {
      if (player?.id !== id) return;
      clearTimeout(timeout);
      untrack();
      post({ op: "youtubeClose", id });
      player = undefined;
    }
    player = { id, destroy, update({event, info}) {
      if (Number.isFinite(info?.time)) time = info.time;
      if (Number.isFinite(info?.duration)) duration = info.duration;
      if (Number.isFinite(info?.state)) state = info.state;
      if (event === "onReady") { clearTimeout(timeout); events.ready(); }
      if (event === "onError") { clearTimeout(timeout); events.error(info); }
      if (event === "onAutoplayBlocked") events.blocked();
      if (event === "onStateChange") { state = info; events.stateChange(state); }
    } };
    return {
      prepare: (position, playing) => command(playing ? "loadVideoById" : "cueVideoById", position),
      time: () => time, duration: () => duration, running: () => state === 1 || state === 3,
      fullscreen: () => command("fullscreen"),
      play: () => command("playVideo"), pause: () => command("pauseVideo"),
      seek: position => command("seekTo", position), volume: value => command("setVolume", value * 100),
      destroy,
    };
  }
  return {
    volume: { get: () => volume, subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); } },
    youtube: youtube ? { mount: mountYouTube } : undefined,
    microphoneLevel: { open: () => open("microphoneLevel") },
    cameraTile: { open: () => open("cameraTile"), mount },
    dispose() {
      if (closed) return;
      player?.destroy();
      for (const remove of mounts) remove();
      for (const device of devices.values()) device.handle.stop();
      for (const finish of pending.values()) finish("Session closed.");
      post({ op: "dispose" });
      closed = true;
      port.close();
      listeners.clear();
    },
  };
}
