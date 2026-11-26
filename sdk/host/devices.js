/** Capture is owned by the trusted Stellarr page. Every returned resource has a single stop path. */
export async function capture(kind, { file, muted, send, tile, ended, signal }) {
  const mic = kind === "microphoneLevel";
  const player = mic ? document.createElement("audio") : document.createElement("video");
  player.playsInline = true;
  player.muted = !mic;
  player.loop = Boolean(file);
  let stream, fileURL, sourceElement, context, interval, closed = false;
  const cleanup = () => {
    if (closed) return;
    closed = true;
    clearInterval(interval);
    player.pause();
    player.srcObject = null;
    sourceElement?.remove();
    if (fileURL) URL.revokeObjectURL(fileURL);
    stream?.getTracks().forEach(track => track.stop());
    if (context) void context.close().catch(() => {});
    if (!mic) void tile({ op: "clear" }).catch(() => {});
  };
  signal.addEventListener("abort", cleanup, { once: true });
  try {
    // Resume during the trusted allow-button gesture, including for a file input.
    if (mic) { context = new AudioContext(); await context.resume(); }
    if (file) {
      // Decode user-selected bytes only as media. Files without a media MIME
      // type can still be identified by the browser's audio/video decoder.
      const type = file.type.startsWith(mic ? "audio/" : "video/") ? file.type : "application/octet-stream";
      fileURL = URL.createObjectURL(file.slice(0, file.size, type));
      const originPrefix = `blob:${location.origin}/`;
      if (!fileURL.startsWith(originPrefix)) throw new Error("Expected a local media object URL.");
      // The File API gives the object a UUID in this origin's blob registry.
      // Pass only that path segment to the decoder.
      const objectId = fileURL.slice(originPrefix.length);
      sourceElement = document.createElement("source");
      sourceElement.src = originPrefix + encodeURIComponent(objectId);
      sourceElement.addEventListener("error", () => {
        if (!closed) { player.pause(); ended(); }
      }, { once: true });
      player.append(sourceElement);
    } else {
      stream = await navigator.mediaDevices.getUserMedia(mic ? { audio: true } : { video: { width: 320, height: 240 }, audio: false });
      if (signal.aborted) { stream.getTracks().forEach(track => track.stop()); throw new Error("Device request closed."); }
      for (const track of stream.getTracks()) track.addEventListener("ended", ended, { once: true });
      if (!mic) player.srcObject = stream;
    }
    if (signal.aborted) throw new Error("Device request closed.");
    player.addEventListener("error", ended, { once: true });
    if (mic) {
      const source = file ? context.createMediaElementSource(player) : context.createMediaStreamSource(stream);
      const analyser = context.createAnalyser();
      analyser.fftSize = 512;
      source.connect(analyser);
      const silence = context.createGain();
      silence.gain.value = 0;
      analyser.connect(silence).connect(context.destination);
      const bytes = new Uint8Array(analyser.fftSize);
      if (file) await player.play();
      if (signal.aborted) throw new Error("Device request closed.");
      interval = setInterval(() => {
        analyser.getByteTimeDomainData(bytes);
        let sum = 0;
        for (const byte of bytes) sum += ((byte - 128) / 128) ** 2;
        send({ op: "level", value: muted() ? 0 : Math.min(1, Math.sqrt(sum / bytes.length) * 2) });
      }, 50);
    } else {
      await player.play();
      if (signal.aborted) throw new Error("Device request closed.");
      const canvas = document.createElement("canvas");
      canvas.width = 240;
      canvas.height = 180;
      const drawing = canvas.getContext("2d");
      let busy = false;
      interval = setInterval(async () => {
        if (busy || player.readyState < 2) return;
        busy = true;
        try {
          drawing.drawImage(player, 0, 0, canvas.width, canvas.height);
          await tile({ op: "put", image: canvas.toDataURL("image/jpeg", 0.55) });
          if (closed) await tile({ op: "clear" });
        } catch (error) {
          // The relay may drop a frame when two uploads arrive close together.
          if (!closed && error?.status !== 429) ended();
        }
        finally { busy = false; }
      }, 250);
    }
    return cleanup;
  } catch (error) { cleanup(); throw error; }
}
