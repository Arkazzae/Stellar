/** The parent owns device access and camera pixels. This side only handles SDK events and layout. */
// The instance serves this module at a reserved URL. In a same-host
// sandbox the parent gives each launch a separate handshake nonce.
export function connectPort(type, port) {
  const nonce = globalThis.origin === "null" && /^#stellarr=([0-9a-f-]{36})$/i.exec(location.hash)?.[1];
  if (nonce) window.parent.postMessage({ type, nonce }, new URL(import.meta.url).origin, [port]);
  else window.parent.postMessage(type, "*", [port]);
}

// A provider speaks its own postMessage protocol. Keep its window and origin paired.
export function embedUrl(input, hosts) {
  const url = new URL(input);
  if (url.protocol !== "https:" || url.port || url.username || url.password || !hosts.includes(url.hostname))
    throw new Error("Expected an HTTPS embed URL from a host declared in connects.");
  return url;
}

export function attachEmbed(container, url, { title = url.hostname, onLoad, onMessage }) {
  const doc = container.ownerDocument;
  const win = doc.defaultView;
  const frame = doc.createElement("iframe");
  frame.title = title;
  frame.allow = "autoplay; encrypted-media; fullscreen; picture-in-picture; camera 'none'; microphone 'none'; display-capture 'none'";
  frame.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms allow-presentation");
  frame.referrerPolicy = "strict-origin";
  frame.style.cssText = "display:block;width:100%;height:100%;border:0";
  frame.onload = () => onLoad?.();
  function receive(event) {
    if (event.source === frame.contentWindow && event.origin === url.origin) onMessage?.(event.data);
  }
  win.addEventListener("message", receive);
  frame.src = url.href;
  container.append(frame);
  return {
    setVisible(visible) { frame.style.visibility = visible ? "visible" : "hidden"; },
    postMessage(data) { frame.contentWindow?.postMessage(data, url.origin); },
    destroy() { win.removeEventListener("message", receive); frame.onload = null; frame.remove(); },
  };
}

export function createMedia({ context, permissions = [], volume: initialVolume = 1, embedHosts = [], hostedEmbeds = false }) {
  const channel = new MessageChannel();
  const port = channel.port1;
  const pending = new Map();
  const devices = new Map();
  const mounts = new Set();
  const listeners = new Set();
  let volume = initialVolume ?? 1;
  const embeds = new Map();
  let closed = false;
  let sequence = 0;
  const post = data => { if (!closed) port.postMessage(data); };

  port.onmessage = ({ data }) => {
    if (!data) return;
    if (data.op === "volume" && Number.isFinite(data.value)) {
      volume = Math.max(0, Math.min(1, data.value));
      for (const fn of listeners) fn(volume);
    } else if (data.op === "embedLoaded") {
      embeds.get(data.id)?.onLoad?.();
    } else if (data.op === "embedMessage") {
      embeds.get(data.id)?.onMessage?.(data.value);
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
    let visible = true, stopped = false;
    function position() {
      const r = visible && element.isConnected ? element.getBoundingClientRect() : null;
      const bounds = [r?.left ?? 0, r?.top ?? 0, r?.width ?? 0, r?.height ?? 0, innerWidth, innerHeight];
      const key = bounds.join(",");
      if (key !== previous) {
        report(bounds);
        previous = key;
      }
      if (visible) raf = requestAnimationFrame(position);
    }
    position();
    return {
      setVisible(value) {
        if (stopped || visible === value) return;
        visible = value;
        cancelAnimationFrame(raf);
        position();
      },
      stop() { stopped = true; cancelAnimationFrame(raf); },
    };
  }
  function mount(participant, element) {
    if (closed || !permissions.includes("media.cameraTile") || !(element instanceof Element))
      return Object.assign(() => {}, { setVisible() {} });
    const id = ++sequence;
    const layout = trackBounds(element, bounds => post({ op: "mount", id, participant, bounds }));
    const remove = () => { layout.stop(); post({ op: "unmount", id }); mounts.delete(remove); };
    mounts.add(remove);
    return Object.assign(remove, { setVisible: layout.setVisible });
  }
  function mountEmbed(input, element, options = {}) {
    if (closed || !(element instanceof Element)) throw new Error("Expected an active session and an embed slot.");
    const url = embedUrl(input, embedHosts);
    const id = ++sequence;
    let handle;
    if (hostedEmbeds) {
      post({ op: "embedOpen", id, url: url.href, title: options.title ?? url.hostname });
      const layout = trackBounds(element, bounds => post({ op: "embedLayout", id, bounds }));
      handle = {
        setVisible: layout.setVisible,
        postMessage: value => post({ op: "embedPost", id, value }),
        destroy() { layout.stop(); post({ op: "embedClose", id }); },
      };
    } else {
      handle = attachEmbed(element, url, options);
    }
    function destroy() {
      if (embeds.delete(id)) handle.destroy();
    }
    embeds.set(id, { onMessage: options.onMessage, onLoad: options.onLoad, destroy });
    return { setVisible: handle.setVisible, postMessage: handle.postMessage, destroy };
  }
  return {
    volume: { get: () => volume, subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); } },
    embed: { mount: mountEmbed },
    microphoneLevel: { open: () => open("microphoneLevel") },
    cameraTile: { open: () => open("cameraTile"), mount },
    dispose() {
      if (closed) return;
      for (const embed of embeds.values()) embed.destroy();
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
