import { embedUrl } from "../media.js";
import { capture } from "./devices.js";

const kinds = ["microphoneLevel", "cameraTile"];
export function bindMediaHost({ frame, origin, nonce, embedHosts = [], title, permissions = [], controls, tile, camera, cloneLocalCamera, volume, muted = () => false, fixtures = false, language = "en", overlay, ask, labels = {} }) {
  const words = (en, polish) => (typeof language === "function" ? language() : language).startsWith("pl") ? polish : en;
  const deviceName = kind => kind === "microphoneLevel" ? words("Microphone", "Mikrofon") : words("Camera", "Kamera");
  const deviceFiles = new Map(), active = new Map(), mounts = new Map();
  let transport;
  const cameraTransport = () => transport ??= camera?.();
  let port, prompt, simMuted = false, disposed = false, requestingImages = false;
  function send(data) { port?.postMessage(data); }
  const embeds = new Map();
  function stop(kind, reason) {
    const device = active.get(kind);
    if (!device) return;
    active.delete(kind);
    device.controller.abort();
    device.button.remove();
    send({ op: "ended", kind, reason });
  }
  const button = (text, fn) => {
    const node = document.createElement("button");
    node.type = "button";
    node.textContent = text;
    node.onclick = fn;
    return node;
  };
  const toolbar = document.createElement("span");
  toolbar.className = "activity-device-controls";
  controls.append(toolbar);
  for (const kind of kinds) {
    if (!fixtures || !permissions.includes("media." + kind)) continue;
    const label = document.createElement("label");
    const file = document.createElement("input");
    file.type = "file";
    file.accept = kind === "microphoneLevel" ? "audio/*" : "video/*";
    label.append(deviceName(kind) + words(" fixture ", " z pliku "), file);
    file.onchange = () => { stop(kind, "device-changed"); deviceFiles.set(kind, file.files?.[0]); };
    toolbar.append(label);
    if (kind === "microphoneLevel") {
      const toggle = button(words("Mute mic", "Wycisz mikrofon"), () => {
        simMuted = !simMuted;
        toggle.setAttribute("aria-pressed", String(simMuted));
        toggle.textContent = simMuted ? words("Unmute mic", "Włącz mikrofon") : words("Mute mic", "Wycisz mikrofon");
      });
      toolbar.append(toggle);
    }
  }
  // The page that hosts the frame may ask in its own words (`ask`); without
  // that, the host asks in a dialog of its own.
  function open(kind, id) {
    const requester = port;
    const reply = (id, error) => requester.postMessage({ op: "reply", id, error });
    if (!permissions.includes("media." + kind) || !kinds.includes(kind)) return reply(id, "Permission unavailable.");
    if (active.has(kind)) return reply(id, active.get(kind).ready ? undefined : "Device request is pending.");
    if (prompt) return reply(id, "Another device request is pending.");
    let settled = false;
    let dismiss = () => {};
    const settle = () => {
      if (settled) return false;
      settled = true;
      dismiss();
      prompt = undefined;
      return true;
    };
    const deny = () => { if (settle()) reply(id, "Permission denied."); };
    const allow = async () => {
      if (!settle()) return;
      const controller = new AbortController();
      const end = () => { if (active.get(kind)?.controller === controller) stop(kind, "unavailable"); };
      const control = button(labels.stop?.[kind] ?? words("Stop ", "Zatrzymaj ") + deviceName(kind), () => stop(kind, "stopped"));
      toolbar.append(control);
      const device = { button: control, controller, id, ready: false };
      active.set(kind, device);
      try {
        await capture(kind, { file: deviceFiles.get(kind), muted: () => simMuted || muted(), send: data => requester.postMessage(data), tile, cloneLocalCamera, camera: kind === "cameraTile" ? cameraTransport() : undefined, ended: end, signal: controller.signal });
        if (controller.signal.aborted) throw new Error("Device stopped.");
        device.ready = true;
        reply(id);
      } catch (error) { if (active.get(kind)?.controller === controller) end(); reply(id, error.message || "Device unavailable."); }
    };
    prompt = { id, kind, cancel: deny };
    if (ask) {
      // A cancelled question is withdrawn from the page, and a late answer to
      // it changes nothing.
      const question = new AbortController();
      dismiss = () => question.abort();
      Promise.resolve()
        .then(() => ask(kind, question.signal))
        .then(yes => (yes && !disposed ? allow() : deny()), deny);
      return;
    }
    const dialog = document.createElement("dialog");
    dialog.style.cssText = "max-width:26rem;padding:1.4rem;border-radius:12px;font:16px system-ui";
    const explanation = document.createElement("p");
    explanation.textContent = title + (kind === "microphoneLevel"
      ? words(" wants to read your microphone level for this game.", " prosi o poziom głośności mikrofonu na potrzeby gry.")
      : words(" wants to show your camera to everyone in this activity.", " chce pokazać Twoją kamerę uczestnikom tej aktywności."));
    dismiss = () => dialog.remove();
    dialog.oncancel = event => { event.preventDefault(); deny(); };
    dialog.append(explanation, button(words("Deny", "Odmów"), deny), button(words("Allow", "Zezwól"), allow));
    document.body.append(dialog);
    dialog.showModal();
  }
  function removeMount(id) {
    const item = mounts.get(id);
    item?.unwatch?.();
    item?.image.remove();
    mounts.delete(id);
  }
  // Hosted media follows the activity viewport, including in fullscreen.
  function layout(node, bounds) {
    const r = frame.getBoundingClientRect();
    const base = overlay ? overlay.getBoundingClientRect() : { left: 0, top: 0 };
    const [x,y,w,h,vw,vh] = bounds;
    const sx = r.width / vw, sy = r.height / vh;
    const left = Math.max(0, x), top = Math.max(0, y);
    const width = Math.max(0, Math.min(vw, x+w)-left), height = Math.max(0, Math.min(vh, y+h)-top);
    Object.assign(node.style, { left: r.left - base.left + left*sx + "px", top: r.top - base.top + top*sy + "px",
      width: width*sx + "px", height: height*sy + "px", visibility: width && height ? "visible" : "hidden" });
  }
  function validBounds(bounds) {
    return Array.isArray(bounds) && bounds.length === 6 && bounds.every(Number.isFinite) && bounds[4] > 0 && bounds[5] > 0;
  }
  function openEmbed({ id, url: input, title }) {
    if (!nonce || !Number.isSafeInteger(id) || embeds.has(id)) return;
    let url;
    try { url = embedUrl(input, embedHosts); } catch { return; }
    if (url.origin === location.origin) return;
    const shell = document.createElement("iframe");
    const shellUrl = new URL("_stellarr/embed.html", frame.src);
    shell.title = typeof title === "string" ? title : url.hostname;
    shell.allow = "autoplay; encrypted-media; fullscreen; picture-in-picture";
    shell.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms allow-presentation");
    shell.style.cssText = (overlay ? "position:absolute" : "position:fixed") + ";width:0;height:0;border:0;visibility:hidden";
    const item = { frame: shell, destroy() {
      shell.onload = null;
      item.provider?.destroy();
      shell.remove();
      embeds.delete(id);
    } };
    shell.onload = () => {
      const doc = shell.contentDocument;
      if (doc?.URL !== shellUrl.href || !doc.defaultView.mountStellarrEmbed) return;
      shell.onload = null;
      item.provider = doc.defaultView.mountStellarrEmbed(doc.body, url, { title: shell.title,
        onLoad: () => send({ op: "embedLoaded", id }),
        onMessage: value => send({ op: "embedMessage", id, value }),
      });
    };
    shell.src = shellUrl.href;
    embeds.set(id, item);
    (overlay ?? document.body).append(shell);
  }
  function mount(data) {
    if (!permissions.includes("media.cameraTile") || !Number.isSafeInteger(data.id)) return;
    if (!validBounds(data.bounds) || typeof data.participant !== "string") return removeMount(data.id);
    let item = mounts.get(data.id);
    if (!item) {
      if (mounts.size >= 16) return;
      const image = document.createElement(camera ? "video" : "img");
      const name = labels.camera ?? words("Activity camera", "Kamera w aktywności");
      if (camera) {
        image.autoplay = image.muted = image.playsInline = true;
        image.setAttribute("aria-label", name);
      } else image.alt = name;
      image.style.cssText = (overlay ? "position:absolute" : "position:fixed") + ";pointer-events:none;object-fit:cover;z-index:5;border-radius:8px";
      image.hidden = true;
      (overlay ?? document.body).append(image);
      mounts.set(data.id, item = { image });
    }
    if (camera && data.participant !== item.participant) {
      item.unwatch?.();
      item.unwatch = cameraTransport().watch(data.participant, stream => {
        if (item.image.srcObject !== stream) item.image.srcObject = stream;
        item.image.hidden = !stream;
      });
    }
    Object.assign(item, { participant: data.participant, bounds: data.bounds });
    layout(item.image, item.bounds);
  }
  // Only development previews use an image relay. WebRTC tiles follow
  // track changes, and layout changes are independent of the video frames.
  const relayout = () => {
    for (const item of mounts.values()) layout(item.image, item.bounds);
    for (const item of embeds.values()) if (item.bounds) layout(item.frame, item.bounds);
  };
  const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(relayout);
  observer?.observe(frame);
  window.addEventListener("scroll", relayout, true);
  const refresh = !camera && tile && setInterval(async () => {
    if (!mounts.size || requestingImages) return;
    requestingImages = true;
    try {
      const images = await tile({ op: "get", ids: [...new Set([...mounts.values()].map(item => item.participant))] });
      for (const item of mounts.values()) {
        layout(item.image, item.bounds);
        const source = images[item.participant];
        item.image.hidden = !source;
        if (source && item.image.getAttribute("src") !== source) item.image.src = source;
      }
    } catch { for (const item of mounts.values()) item.image.hidden = true; }
    finally { requestingImages = false; }
  }, 250);
  const volumeChanged = () => send({ op: "volume", value: Number(volume.value) / 100 });
  volume.addEventListener("input", volumeChanged);
  function reset() {
    for (const item of embeds.values()) item.destroy();
    prompt?.cancel();
    for (const kind of active.keys()) stop(kind, "closed");
    for (const id of mounts.keys()) removeMount(id);
    transport?.dispose();
    transport = undefined;
    port?.close();
    port = undefined;
  }
  function bind(event) {
    if (event.source !== frame.contentWindow || event.origin !== origin || !event.ports[0]) return;
    if (nonce ? port || event.data?.type !== "stellarr:media:v1" || event.data?.nonce !== nonce : event.data !== "stellarr:media:v1") return;
    reset();
    port = event.ports[0];
    port.onmessage = ({ data }) => {
      if (disposed || !data) return;
      switch (data.op) {
        case "embedOpen": return openEmbed(data);
        case "embedClose": return embeds.get(data.id)?.destroy();
        case "embedPost": return embeds.get(data.id)?.provider?.postMessage(data.value);
        case "embedLayout": {
          const item = embeds.get(data.id);
          if (item && validBounds(data.bounds)) {
            item.bounds = data.bounds;
            layout(item.frame, item.bounds);
          }
          return;
        }
        case "open": return open(data.kind, data.id);
        case "stop": return stop(data.kind, "stopped");
        case "cancel":
          if (prompt?.id === data.id) prompt.cancel();
          for (const [kind, device] of active) if (device.id === data.id) stop(kind, "stopped");
          return;
        case "mount": return mount(data);
        case "unmount": return removeMount(data.id);
        case "dispose": return reset();
      }
    };
    volumeChanged();
  }
  window.addEventListener("message", bind);
  window.addEventListener("pagehide", reset);
  return {
    stopCamera() {
      if (prompt?.kind === "cameraTile") prompt.cancel();
      stop("cameraTile", "camera-off");
    },
    dispose() {
      // Nothing that goes wrong while letting go of the devices may leave the
      // image poll or the listeners behind.
      disposed = true;
      clearInterval(refresh);
      observer?.disconnect();
      window.removeEventListener("scroll", relayout, true);
      volume.removeEventListener("input", volumeChanged);
      window.removeEventListener("message", bind);
      window.removeEventListener("pagehide", reset);
      try {
        reset();
      } finally {
        for (const id of [...mounts.keys()]) removeMount(id);
        toolbar.remove();
      }
    },
  };
}
