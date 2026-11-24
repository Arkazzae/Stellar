// What one server answers about voice and GIFs under the settings it was
// started with: found an instance, ask for the bootstrap, say its rtc part and
// check the relay's credentials against the secret, then ask for the GIF
// configuration. Used by bootstrap-settings.sh.
//
//   node bootstrap-settings.mjs --base URL --setup CODE --secret S

import { createHmac } from "node:crypto";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({ options: { base: { type: "string" }, setup: { type: "string" }, secret: { type: "string" } } });
const H = { "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json" };
const register = await fetch(args.base + "/api/auth/register", {
  method: "POST", headers: H,
  body: JSON.stringify({ username: "ola", displayName: "Ola", email: "ola@example.test", password: "Corona!2026-long", setupCode: args.setup }),
});
const cookie = register.headers.getSetCookie().map((l) => l.split(";")[0]).join("; ");
const ask = (path) => fetch(args.base + path, { headers: { ...H, Cookie: cookie } });

const bootstrap = await ask("/api/bootstrap");
const text = await bootstrap.text();
console.log(`bootstrap ${bootstrap.status}`);
if (bootstrap.status === 200) {
  const rtc = JSON.parse(text).rtc;
  for (const server of rtc.iceServers) {
    if (server.username) {
      const want = createHmac("sha1", args.secret).update(server.username).digest("base64");
      const left = Number(server.username.split(":")[0]) - Date.now() / 1000;
      console.log(`  credential ${want === server.credential ? "matches" : "DIFFERS"}, ${left > 3590 && left <= 3600 ? "an hour" : left}`);
      server.username = "<username>";
      server.credential = "<credential>";
    }
  }
  console.log("  " + JSON.stringify(rtc));
} else console.log("  " + text.slice(0, 200).replace(/"traceId":"[^"]*"/, '"traceId":"<trace>"'));
const giphy = await ask("/api/config/giphy");
console.log(`giphy ${giphy.status} ${await giphy.text()}`);
