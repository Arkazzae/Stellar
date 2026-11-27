// The test page runs in an installed activity frame and reports the bytes the
// instance actually served. Compare both modules with the public npm archive.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { launch } from "../../web/bench/cdp.mjs";

const [origin, sdk] = process.argv.slice(2);
const browser = await launch({ flags: ["--ignore-certificate-errors", "--no-proxy-server"] });
try {
  const page = await browser.newPage();
  await page.goto(origin + "/login");
  const status = await page.eval(`fetch("/api/auth/login", {
    method: "POST", headers: {"Content-Type":"application/json","X-Stellarr-Client":"web","X-Stellarr-Client-Version":"2.0.0"},
    body: JSON.stringify({username:"olga", password:"Voice-Call-Check-1"})
  }).then(r => r.status)`);
  assert.equal(status, 200, "voice smoke test owner login");
  await page.goto(origin);
  const launcher = `document.querySelector('[aria-label="Add a server and more"]')`;
  const activities = `document.querySelector('.srv-menu:not([hidden]) button[aria-label="Solo activities"]')`;
  const start = '[...document.querySelectorAll(".act-card")].find(card => card.querySelector(".act-card-name")?.textContent === "Release check")';
  for (const selector of [launcher, activities]) {
    await page.waitFor(selector);
    await page.eval(selector + ".click()");
  }
  await page.eval(`window.addEventListener("message", e => {
    const frame = document.querySelector(".activity-viewport iframe");
    if (frame && e.source === frame.contentWindow && e.origin === "null" &&
        new URL(frame.src).origin === location.origin && e.data?.releaseCheck) {
      window.releaseResult = e.data.releaseCheck;
    }
  })`);
  await page.waitFor(start);
  await page.eval(start + ".click()");
  await page.waitFor("window.releaseResult");
  const result = await page.eval("window.releaseResult");
  assert.equal(result.mode, "personal");
  for (const name of ["client.js", "media.js"]) {
    const expected = createHash("sha256").update(await readFile(join(sdk, name))).digest("hex");
    assert.equal(result[name], expected, name + " must match the published SDK");
  }
  console.log("Personal activity connected. Both served SDK modules match npm.");
} finally {
  await browser.close();
}
