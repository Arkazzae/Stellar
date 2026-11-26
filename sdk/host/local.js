import { bindMediaHost } from "./media-host.js";

const config = await (await fetch("/_stellarr/config")).json();
const frame = document.querySelector("#activity");
const slider = document.querySelector("#volume");
document.title = config.name + " · Stellarr";
document.querySelector("#name").textContent = config.name;
frame.title = config.name;
const key = "stellarr.preview.player";
const resume = performance.getEntriesByType("navigation")[0]?.type === "reload";
const token = resume && sessionStorage.getItem(key) || crypto.randomUUID();
sessionStorage.setItem(key, token);
async function call(route, action, keepalive = false) {
  const response = await fetch("/_stellarr/dev/" + route, { method: "POST", keepalive,
    headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token, action }) });
  if (!response.ok) throw Object.assign(new Error(await response.text()), { status: response.status });
  return response.json();
}
let connected, frameId;
function disconnect() {
  if (frameId) void call("room", { op: "disconnect", frame: frameId }, true).catch(() => {});
  frameId = undefined;
  connected?.close();
}
window.addEventListener("pagehide", disconnect);
window.addEventListener("message", event => {
  if (event.source !== frame.contentWindow || event.origin !== config.frame || event.data !== "stellarr:dev-connect:v1" || !event.ports[0]) return;
  disconnect();
  const id = frameId = crypto.randomUUID();
  const port = connected = event.ports[0];
  port.onmessage = async ({ data }) => {
    if (!data || !Number.isSafeInteger(data.id) || !data.input) return;
    try {
      const result = await call("room", { ...data.input, frame: id });
      if (data.input.op === "open") Object.assign(result, { standalone: true, locale: navigator.language, volume: Number(slider.value)/100 });
      port.postMessage({ id: data.id, value: result });
    } catch (error) { port.postMessage({ id: data.id, error: error.message, status: error.status }); }
  };
});
bindMediaHost({ frame, origin: config.frame, title: config.name, permissions: config.permissions, controls: document.querySelector("#devices"),
  volume: slider, tile: action => call("tile", action), fixtures: true });
frame.src = config.frame + "/";
