// The bootstrap and native installer can read server releases with a plain HTTPS client.
const registry = "https://ghcr.io";
const repository = "arkazzae/stellarr";
const version = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*)?$/;
const series = /^(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const asset = /^stellarr-installer(?:-(?:aarch64|x86_64)-(?:apple-darwin|unknown-linux-musl)|\.sha256)$/;

export async function onRequest({ request, params }) {
  const { method } = request;
  if (method !== "GET" && method !== "HEAD") return new Response(null, { status: 405, headers: { Allow: "GET, HEAD" } });
  const [track, file, ...extra] = params.path || [];
  const exact = version.test(track);
  if (!track || track.length > 128 || extra.length ||
      !(exact || track === "latest" || series.test(track)) || (file !== undefined && !(exact && asset.test(file)))) {
    return new Response(null, { status: 404 });
  }
  const auth = await fetch(registry + "/token?service=ghcr.io&scope=repository:" + repository + ":pull");
  if (!auth.ok) return new Response(null, { status: 502 });
  const { token } = await auth.json();
  const headers = { Authorization: "Bearer " + token, Accept: "application/vnd.oci.image.manifest.v1+json" };
  const base = registry + "/v2/" + repository;
  let tag = "installer-" + track;
  if (tag.length > 128) {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(track));
    tag = "installer-" + Array.from(new Uint8Array(digest), b => b.toString(16).padStart(2, "0")).join("");
  }
  const result = await fetch(base + "/manifests/" + tag, { headers });
  if (!result.ok) return new Response(null, { status: result.status === 404 ? 404 : 502 });
  const manifest = await result.json();
  const resolved = manifest.annotations?.["org.opencontainers.image.version"];
  if (manifest.artifactType !== "application/vnd.stellarr.server-installers.v1" || !version.test(resolved) ||
      (exact ? resolved !== track : resolved.includes("-") || (track !== "latest" && !resolved.startsWith(track + ".")))) {
    return new Response(null, { status: 502 });
  }
  if (!file) return new Response(method === "HEAD" ? null : resolved + "\n", {
    headers: { "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" },
  });
  const layer = manifest.layers.find(item => item.annotations?.["org.opencontainers.image.title"] === file);
  if (!layer || !/^sha256:[0-9a-f]{64}$/.test(layer.digest)) return new Response(null, { status: 404 });
  const blob = await fetch(base + "/blobs/" + layer.digest, { method, headers, redirect: "manual" });
  if ([301, 302, 303, 307, 308].includes(blob.status)) {
    const location = blob.headers.get("Location");
    if (!location || new URL(location, registry).protocol !== "https:") return new Response(null, { status: 502 });
    return new Response(null, { status: 302, headers: { Location: new URL(location, registry).href, "Cache-Control": "no-store" } });
  }
  if (!blob.ok) return new Response(null, { status: 502 });
  return new Response(blob.body, { headers: { "Content-Type": file.endsWith(".sha256") ? "text/plain" : "application/octet-stream" } });
}
