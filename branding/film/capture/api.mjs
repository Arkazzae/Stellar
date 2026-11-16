// In-page API helper: fetch from a page on the web origin, so cookies and origin are the app's own.
export const BASE = process.env.STELLARR_FILM_WEB || "http://127.0.0.1:3100";

export async function installApi(page) {
  await page.eval(`(() => {
    window.__api = async (method, path, body, serverId) => {
      const h = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", Accept: "application/json" };
      if (body !== undefined && !(body instanceof FormData)) h["Content-Type"] = "application/json";
      if (serverId) h["X-Stellarr-Server"] = serverId;
      const r = await fetch(path, { method, headers: h, credentials: "same-origin", body: body === undefined ? undefined : body instanceof FormData ? body : JSON.stringify(body) });
      const text = await r.text();
      let json; try { json = JSON.parse(text); } catch { json = text; }
      return { status: r.status, json };
    };
    return true;
  })()`);
}

export async function api(page, method, path, body, serverId) {
  // an undefined body arrives as undefined (an empty call argument), so a GET sends no body
  const r = await page.call((method, path, body, serverId) => window.__api(method, path, body, serverId), method, path, body, serverId ?? null);
  if (r.status >= 400) throw new Error(`${method} ${path} -> ${r.status} ${JSON.stringify(r.json).slice(0, 400)}`);
  return r.json;
}
