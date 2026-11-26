import { capture } from "./devices.js";

import { createYouTubeHost } from "./youtube.js";

const kinds = ["microphoneLevel", "cameraTile"];
export function bindMediaHost({ frame, origin, nonce, youtube, title, permissions = [], controls, tile, volume, muted = () => false, fixtures = false, language = "en" }) {
  const pl = language.startsWith("pl");
  const words = (en, polish) => pl ? polish : en;
  const deviceName = kind => kind === "microphoneLevel" ? words("Microphone", "Mikrofon") : words("Camera", "Kamera");
  const deviceFiles = new Map(), active = new Map(), mounts = new Map();
  let port, prompt, simMuted = false, disposed = false, requestingImages = false;
  function send(data) { port?.postMessage(data); }
  const player = youtube && nonce ? createYouTubeHost(frame, send) : undefined;
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
  function open(kind, id) {
    const requester = port;
    const reply = (id, error) => requester.postMessage({ op: "reply", id, error });
    if (!permissions.includes("media." + kind) || !kinds.includes(kind)) return reply(id, "Permission unavailable.");
    if (active.has(kind)) return reply(id, active.get(kind).ready ? undefined : "Device request is pending.");
    if (prompt) return reply(id, "Another device request is pending.");
    const dialog = document.createElement("dialog");
    dialog.style.cssText = "max-width:26rem;padding:1.4rem;border-radius:12px;font:16px system-ui";
    const explanation = document.createElement("p");
    explanation.textContent = title + (kind === "microphoneLevel"
      ? words(" wants to read your microphone level for this game.", " prosi o poziom głośności mikrofonu na potrzeby gry.")
      : words(" wants to show your camera to everyone in this activity.", " chce pokazać Twoją kamerę uczestnikom tej aktywności."));
    const deny = () => { dismiss(); reply(id, "Permission denied."); };
    const dismiss = () => { dialog.remove(); prompt = undefined; };
    prompt = { id, kind, cancel: deny };
    dialog.oncancel = event => { event.preventDefault(); deny(); };
    dialog.append(explanation, button(words("Deny", "Odmów"), deny), button(words("Allow", "Zezwól"), async () => {
      dismiss();
      const controller = new AbortController();
      const end = () => { if (active.get(kind)?.controller === controller) stop(kind, "unavailable"); };
      const control = button(words("Stop ", "Zatrzymaj ") + deviceName(kind), () => stop(kind, "stopped"));
      toolbar.append(control);
      const device = { button: control, controller, id, ready: false };
      active.set(kind, device);
      try {
        await capture(kind, { file: deviceFiles.get(kind), muted: () => simMuted || muted(), send: data => requester.postMessage(data), tile, ended: end, signal: controller.signal });
        if (controller.signal.aborted) throw new Error("Device stopped.");
        device.ready = true;
        reply(id);
      } catch (error) { if (active.get(kind)?.controller === controller) end(); reply(id, error.message || "Device unavailable."); }
    }));
    document.body.append(dialog);
    dialog.showModal();
  }
  function removeMount(id) { mounts.get(id)?.image.remove(); mounts.delete(id); }
  function layout({ image, bounds }) {
    const r = frame.getBoundingClientRect();
    const [x,y,w,h,vw,vh] = bounds;
    const sx = r.width / vw, sy = r.height / vh;
    const left = Math.max(0, x), top = Math.max(0, y);
    const width = Math.max(0, Math.min(vw, x+w)-left), height = Math.max(0, Math.min(vh, y+h)-top);
    Object.assign(image.style, { left: r.left + left*sx + "px", top: r.top + top*sy + "px",
      width: width*sx + "px", height: height*sy + "px" });
  }
  function mount(data) {
    if (!permissions.includes("media.cameraTile") || !Number.isSafeInteger(data.id)) return;
    if (!Array.isArray(data.bounds) || data.bounds.length !== 6 || !data.bounds.every(Number.isFinite) ||
        data.bounds[4] <= 0 || data.bounds[5] <= 0 || typeof data.participant !== "string") return removeMount(data.id);
    let item = mounts.get(data.id);
    if (!item) {
      if (mounts.size >= 16) return;
      const image = document.createElement("img");
      image.alt = words("Activity camera", "Kamera w aktywności");
      image.style.cssText = "position:fixed;pointer-events:none;object-fit:cover;z-index:5;border-radius:8px";
      image.hidden = true;
      document.body.append(image);
      mounts.set(data.id, item = { image });
    }
    Object.assign(item, { participant: data.participant, bounds: data.bounds });
    layout(item);
  }
  const refresh = setInterval(async () => {
    if (!mounts.size || requestingImages) return;
    requestingImages = true;
    try {
      const images = await tile({ op: "get", ids: [...new Set([...mounts.values()].map(item => item.participant))] });
      for (const item of mounts.values()) {
        layout(item);
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
    player?.stop();
    prompt?.cancel();
    for (const kind of active.keys()) stop(kind, "closed");
    for (const id of mounts.keys()) removeMount(id);
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
        case "youtubeOpen": case "youtubeClose": case "youtubeLayout": case "youtubeCommand": return player?.handle(data);
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
      disposed = true;
      reset();
      player?.dispose();
      clearInterval(refresh);
      toolbar.remove();
      volume.removeEventListener("input", volumeChanged);
      window.removeEventListener("message", bind);
      window.removeEventListener("pagehide", reset);
    },
  };
}
