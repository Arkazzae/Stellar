// This player lives inside the activity viewport, outside its opaque sandbox.
// Only the approved YouTube embed receives commands. Its scripts run on YouTube.
const provider = "https://www.youtube-nocookie.com";
const videoId = /^[a-zA-Z0-9_-]{11}$/;

export function createYouTubeHost(activity, send) {
  let active;
  function stop() {
    if (!active) return;
    clearInterval(active.timer);
    active.frame.remove();
    active = undefined;
  }
  function post(message) {
    if (active) active.frame.contentWindow?.postMessage(JSON.stringify({
      ...message, id: active.frame.id, channel: "widget",
    }), provider);
  }
  function receive(event) {
    if (!active || event.source !== active.frame.contentWindow || event.origin !== provider) return;
    let data;
    try { data = JSON.parse(event.data); } catch { return; }
    if (!data || (data.id && data.id !== active.frame.id)) return;
    const info = data.info;
    switch (data.event) {
      case "onReady":
        if (active.ready) return;
        active.ready = true;
        clearInterval(active.timer);
        for (const name of ["onStateChange", "onError", "onAutoplayBlocked"])
          post({ event: "command", func: "addEventListener", args: [name] });
        break;
      case "onError": case "onStateChange": case "onAutoplayBlocked":
      case "infoDelivery": case "initialDelivery": break;
      default: return;
    }
    // Expose playback state only. The provider also sends account and video metadata.
    send({ op: "youtubeEvent", id: active.id, event: data.event,
      info: typeof info === "number" ? info : {
        time: info?.currentTime, duration: info?.duration, state: info?.playerState,
      } });
  }
  function layout(bounds) {
    const viewport = activity.parentElement;
    if (!viewport || !active || !Array.isArray(bounds) || bounds.length !== 6 ||
        !bounds.every(Number.isFinite) || bounds[4] <= 0 || bounds[5] <= 0) return;
    const [x, y, w, h, vw, vh] = bounds;
    const r = activity.getBoundingClientRect(), parent = viewport.getBoundingClientRect();
    const clamp = (n, max) => Math.max(0, Math.min(n, max));
    const left = clamp(x, vw), top = clamp(y, vh);
    Object.assign(active.frame.style, {
      left: r.left - parent.left + r.width * (left / vw) + "px",
      top: r.top - parent.top + r.height * (top / vh) + "px",
      width: r.width * (Math.max(0, clamp(x + w, vw) - left) / vw) + "px",
      height: r.height * (Math.max(0, clamp(y + h, vh) - top) / vh) + "px",
    });
    active.bounds = bounds;
  }
  const resize = new ResizeObserver(() => layout(active?.bounds));
  resize.observe(activity);
  window.addEventListener("message", receive);
  return {
    handle(data) {
      if (!Number.isSafeInteger(data.id)) return;
      if (data.op === "youtubeOpen") {
        if (!videoId.test(data.videoId) || typeof data.videoId !== "string") return;
        stop();
        const frame = document.createElement("iframe");
        frame.id = "stellarr-yt-" + crypto.randomUUID();
        frame.title = "YouTube";
        frame.referrerPolicy = "strict-origin-when-cross-origin";
        frame.allow = "autoplay; encrypted-media; fullscreen; picture-in-picture";
        frame.allowFullscreen = true;
        frame.setAttribute("sandbox", "allow-scripts allow-same-origin allow-presentation");
        frame.style.cssText = "position:absolute;left:0;top:0;width:0;height:0;border:0";
        const params = new URLSearchParams({ enablejsapi: "1", origin: location.origin, playsinline: "1", controls: "0", disablekb: "1", rel: "0" });
        frame.src = provider + "/embed/" + data.videoId + "?" + params;
        active = { id: data.id, videoId: data.videoId, frame,
          timer: setInterval(() => post({ event: "listening" }), 300), ready: false };
        activity.after(frame);
        return;
      }
      if (!active || active.id !== data.id) return;
      if (data.op === "youtubeClose") return stop();
      if (data.op === "youtubeLayout") return layout(data.bounds);
      if (data.op !== "youtubeCommand" || !active.ready) return;
      let args = [];
      switch (data.command) {
        case "fullscreen":
          if (navigator.userActivation.isActive) void active.frame.requestFullscreen().catch(() => {});
          return;
        case "playVideo": case "pauseVideo": break;
        case "seekTo": case "loadVideoById": case "cueVideoById":
          if (!Number.isFinite(data.value) || data.value < 0) return;
          args = data.command === "seekTo" ? [data.value, true] : [active.videoId, data.value];
          break;
        case "setVolume":
          if (!Number.isFinite(data.value)) return;
          args = [Math.max(0, Math.min(100, data.value))];
          break;
        default: return;
      }
      post({ event: "command", func: data.command, args });
    },
    stop,
    dispose() { stop(); resize.disconnect(); window.removeEventListener("message", receive); },
  };
}
