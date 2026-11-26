import assert from "node:assert/strict";
import test from "node:test";
import { connect } from "../client.js";
import { previewProgress } from "../dev/progress.js";
import { createRoom } from "../dev/room.js";

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function eventually(predicate) {
  const until = Date.now() + 4000;
  while (!predicate() && Date.now() < until) await sleep(25);
  assert.ok(predicate(), "The client did not observe the expected room update");
}

test("two embedded SDK clients handle packet loss, ordered state, private messages, commands and teardown", async t => {
  const room = createRoom({ authority: "host", seats: 2 });
  const ports = new Set(), sessions = [];
  const previousWindow = globalThis.window;
  const page = new EventTarget();
  globalThis.window = page;
  t.after(async () => {
    await Promise.allSettled(sessions.map(session => session.close()));
    for (const port of ports) port.close();
    if (previousWindow) globalThis.window = previousWindow;
    else delete globalThis.window;
  });
  const saves = new Map();
  let locale = "pl", device = "phone", dropSave = true;
  let dropWrite = true, dropDecision = true, dropResult = true;
  async function join(id) {
    page.parent = { postMessage(kind, _, [port]) {
      ports.add(port);
      if (kind !== "stellarr:dev-connect:v1") return;
      port.onmessage = ({ data: { id: serial, input } }) => {
        try {
          let value = room(input.op === "open" ? { id, name: id } : { id }, input);
          if (input.op === "open" || input.op === "read") Object.assign(value, { locale, device });
          if (input.op === "open") value.progress = true;
          if (input.op === "loadProgress" || input.op === "saveProgress") {
            value = previewProgress(saves.get(id), input, next => saves.set(id, next));
            if (dropSave && input.op === "saveProgress") { dropSave = false; throw new Error("Lost save acknowledgement"); }
          }
          if (input.op === "open") Object.assign(value, { standalone: true, volume: 0.4 });
          if (input.op === "write" && dropWrite) { dropWrite = false; throw new Error("Lost write reply"); }
          if (input.op === "decide" && dropDecision) { dropDecision = false; throw new Error("Lost decision reply"); }
          if (input.op === "submit" && value.result && dropResult) { dropResult = false; throw new Error("Lost result reply"); }
          port.postMessage({ id: serial, value });
        } catch (error) { port.postMessage({ id: serial, error: error.message, status: error.status ?? 0 }); }
      };
    } };
    const session = await connect({ initialState: { score: 0 } });
    sessions.push(session);
    return session;
  }
  const alice = await join("alice"), bob = await join("bob");
  assert.equal(alice.context.device, "phone");
  const languages = [], devices = [];
  alice.locale.subscribe(value => languages.push(value));
  alice.device.subscribe(value => devices.push(value));
  locale = "en"; device = "tablet";
  room.updateProfile({ id: "alice", name: "Alicja", avatar: "/portrait" });
  await eventually(() => alice.context.me.name === "Alicja" && bob.participants.get()[0].avatar === "/portrait");
  assert.deepEqual(languages, ["pl", "en"]);
  assert.deepEqual(devices, ["phone", "tablet"]);
  assert.equal(alice.context.locale, "en");
  assert.equal(alice.context.device, "tablet");
  assert.ok(alice.capabilities.includes("progress"));
  const circular = {};
  circular.self = circular;
  await assert.rejects(alice.progress.save(circular, 0), TypeError);
  assert.equal(await alice.progress.save({ level: 3 }, 0), 1);
  assert.deepEqual(await alice.progress.load(), { data: { level: 3 }, revision: 1 });
  assert.deepEqual(await bob.progress.load(), { data: null, revision: 0 });
  await assert.rejects(alice.progress.save({ level: 4 }, 0), { status: 409 });
  assert.equal(alice.context.standalone, true);
  assert.equal(alice.context.launchMode, "voice");
  assert.equal(alice.media.volume.get(), 0.4);
  assert.ok(Math.abs(alice.now() - Date.now()) < 1000);
  const revisions = [];
  alice.state.subscribe((state, revision) => revisions.push([state.score, revision]));
  assert.equal(await alice.state.set(state => ({ score: state.score + 1 })), 1);
  const handler = (data, { from }, state, privateState) => {
    assert.equal(from, "bob");
    assert.deepEqual(privateState.assignments, {});
    return { patch: { score: state.score + data }, result: "accepted", assignments: { bob: "for-bob" } };
  };
  alice.commands.handle("score", handler);
  const result = await bob.commands.submit("score", 2, { id: "turn-1" });
  assert.equal(result, "accepted");
  assert.equal(bob.state.get().score, 3);
  assert.deepEqual(revisions, [[0, 0], [1, 1], [3, 2]]);
  assert.equal(bob.assignments.get(), "for-bob");
  assert.equal(alice.assignments.get(), null);
  const viewer = await join("viewer");
  assert.deepEqual(viewer.seats.get(), ["alice", "bob"]);
  assert.equal(viewer.assignments.get(), null);
  await assert.rejects(viewer.commands.submit("score", 2), /seat/);
  const delivered = [], leaked = [];
  bob.on("binary", (value, { from }) => delivered.push([Array.from(value), from]));
  viewer.on("binary", value => leaked.push(value));
  assert.ok(alice.send("binary", new Uint8Array([0, 255]), { to: "bob" }));
  await eventually(() => delivered.length === 1);
  assert.deepEqual(delivered, [[[0, 255], "alice"]]);
  assert.deepEqual(leaked, []);
  await alice.host.transfer("bob");
  await eventually(() => bob.host.get() === "bob");
  assert.equal(await bob.commands.submit("score", 2, { id: "turn-1" }), "accepted");
  assert.deepEqual(await bob.assignments.checkpoint(), { revision: 2, state: { score: 3 }, assignments: { bob: "for-bob" } });
  bob.commands.handle("reject", () => { throw new Error("Round is over"); });
  await assert.rejects(alice.commands.submit("reject", false), /Round is over/);
  await viewer.close();
  assert.equal(viewer.connection.get(), "closed");
  assert.equal(viewer.send("x", true), false);
  await eventually(() => bob.participants.get().length === 2);
  room.revoke("alice");
  await eventually(() => alice.connection.get() === "closed");
  assert.equal(alice.send("x", true), false);
});

test("the browser SDK exposes a personal instance session with no voice place", async t => {
  const room = createRoom();
  const previousWindow = globalThis.window;
  const page = new EventTarget();
  let bridge;
  let session;
  globalThis.window = page;
  t.after(async () => {
    await session?.close();
    bridge?.close();
    if (previousWindow) globalThis.window = previousWindow;
    else delete globalThis.window;
  });
  page.parent = { postMessage(kind, _, [port]) {
    if (kind !== "stellarr:dev-connect:v1") return;
    bridge = port;
    port.onmessage = ({ data: { id, input } }) => {
      const value = room({ id: "alice", name: "Alice" }, input);
      if (input.op === "open") Object.assign(value, { launchMode: "personal" });
      port.postMessage({ id, value });
    };
  } };
  session = await connect();
  assert.equal(session.context.launchMode, "personal");
  assert.equal(session.context.standalone, false);
  assert.equal(session.context.place, null);
});

test("a push-capable session delivers messages and drains queued commands between heartbeats", async t => {
  const room = createRoom({ authority: "host" });
  const previousWindow = globalThis.window;
  const page = globalThis.window = new EventTarget();
  const ports = [], reads = [];
  let bridge, session, lostDecision = false, lostSettlement = false;
  t.after(async () => {
    await session?.close();
    for (const port of ports) port.close();
    if (previousWindow) globalThis.window = previousWindow;
    else delete globalThis.window;
  });
  const delivery = {
    message(_user, message) { bridge.postMessage({ update: { message } }); return true; },
    command() { bridge.postMessage({ update: {} }); },
  };
  page.parent = { postMessage(kind, _, [port]) {
    ports.push(port);
    if (kind !== "stellarr:dev-connect:v1") return;
    bridge = port;
    port.onmessage = ({ data: { id, input } }) => {
      const value = room({ id: "alice", name: "Alice" }, input, delivery);
      if (input.op === "open") value.roomEvents = true;
      if (input.op === "read") reads.push(value);
      if (input.op === "decide") {
        if (input.id === "dropped-notice") {
          lostSettlement = true;
        } else {
          port.postMessage({ update: { settled: input.id } });
          if (!lostDecision) {
            lostDecision = true;
            port.postMessage({ id, error: "Decision acknowledgement lost", status: 503 });
            return;
          }
        }
      }
      port.postMessage({ id, value });
    };
  } };
  session = await connect({ initialState: { score: 0 } });
  await eventually(() => reads.length === 1);
  const messages = [];
  session.on("cursor", value => messages.push(value));
  session.send("cursor", 42, { to: "alice" });
  await eventually(() => messages.length > 0);
  await sleep(350);
  assert.deepEqual(messages, [42]);
  assert.equal(reads.length, 1, "idle and transient messages need no short polls");
  assert.equal(reads[0].participants, undefined, "heartbeat omits the known roster");
  session.commands.handle("point", (_data, _command, state) => ({
    patch: { score: state.score + 1 }, result: state.score + 1,
  }));
  const turns = await Promise.all(Array.from({length: 4}, () => session.commands.submit("point", {})));
  assert.deepEqual(turns, [1, 2, 3, 4]);
  assert.equal(session.state.get().score, 4);
  assert.ok(lostDecision);
  assert.equal(await session.commands.submit("point", {}, { id: "dropped-notice" }), 5);
  assert.ok(lostSettlement);
  assert.equal(session.state.get().score, 5);
});
