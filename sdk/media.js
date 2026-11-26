/** The parent owns device access and camera pixels. This side only handles SDK events and layout. */
export function createMedia({ context, permissions = [], volume: initialVolume = 1 }) {
  const channel = new MessageChannel();
  const port = channel.port1;
  const pending = new Map();
  const devices = new Map();
  const mounts = new Set();
  const listeners = new Set();
  let volume = initialVolume ?? 1;
  let closed = false;
  let sequence = 0;
  const post = data => { if (!closed) port.postMessage(data); };

  port.onmessage = ({ data }) => {
    if (!data) return;
    if (data.op === "volume" && Number.isFinite(data.value)) {
      volume = Math.max(0, Math.min(1, data.value));
      for (const fn of listeners) fn(volume);
    } else if (data.op === "level") {
      devices.get("microphoneLevel")?.update(data.value);
    } else if (data.op === "ended") {
      devices.get(data.kind)?.end(data.reason);
    } else if (data.op === "reply") {
      pending.get(data.id)?.(data.error);
    }
  };
  if (window.parent !== window)
    window.parent.postMessage("stellarr:media:v1", "*", [channel.port2]);

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
  function mount(participant, element) {
    if (closed || !permissions.includes("media.cameraTile") || !(element instanceof Element)) return () => {};
    const id = ++sequence;
    let previous = "";
    let raf;
    function position() {
      const r = element.getBoundingClientRect();
      const bounds = [r.left, r.top, r.width, r.height, innerWidth, innerHeight];
      const key = bounds.join(",") + element.isConnected;
      if (key !== previous) {
        post({ op: "mount", id, participant, bounds: element.isConnected ? bounds : [] });
        previous = key;
      }
      raf = requestAnimationFrame(position);
    }
    position();
    const remove = () => { cancelAnimationFrame(raf); post({ op: "unmount", id }); mounts.delete(remove); };
    mounts.add(remove);
    return remove;
  }
  return {
    volume: { get: () => volume, subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); } },
    microphoneLevel: { open: () => open("microphoneLevel") },
    cameraTile: { open: () => open("cameraTile"), mount },
    dispose() {
      if (closed) return;
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
