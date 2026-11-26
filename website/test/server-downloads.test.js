import assert from "node:assert/strict";
import test from "node:test";
import { onRequest } from "../functions/server-downloads/[[path]].js";

function request(path, method = "GET") {
  return onRequest({ request: new Request("https://stellarr.online/server-downloads/" + path.join("/"), { method }), params: { path } });
}

test("only public server tracks and exact-version assets reach the registry", async (t) => {
  const fetch = t.mock.method(globalThis, "fetch", () => { throw new Error("unexpected fetch"); });
  for (const path of [["../latest"], ["latest", "stellarr-installer.sha256"], ["0.1.0", "private"], ["0.1.0", "a", "b"]]) {
    assert.equal((await request(path)).status, 404);
  }
  assert.equal((await request(["latest"], "POST")).status, 405);
  assert.equal(fetch.mock.callCount(), 0);
});

test("aliases must identify a stable server from the requested series", async (t) => {
  let version = "0.1.2";
  t.mock.method(globalThis, "fetch", async url => {
    if (url.includes("/token?")) return Response.json({ token: "test" });
    return Response.json({ artifactType: "application/vnd.stellarr.server-installers.v1", annotations: { "org.opencontainers.image.version": version } });
  });
  assert.equal(await (await request(["0.1"])).text(), "0.1.2\n");
  assert.equal((await request(["0.2"])).status, 502);
  version = "0.1.3-rc.1";
  assert.equal((await request(["latest"])).status, 502);
  assert.equal(await (await request([version])).text(), version + "\n");
});
