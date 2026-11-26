import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import test from "node:test";
import { createRoom } from "../dev/room.js";

function fixture(options = {}) {
  let time = 100;
  const room = createRoom({ ...options, clock: () => time });
  const join = (id, initialState = {}) => {
    const frame = randomUUID();
    // JSON round trips model the wire and prevent the test from mutating authority data.
    const call = input => JSON.parse(JSON.stringify(room({ id, name: id }, { ...input, frame })));
    const opened = call({ op: "open", initialState });
    return { call, opened, read: () => call({ op: "read", after: -1 }) };
  };
  return { join, room, advance: ms => { time += ms; } };
}
const command = { op: "submit", kind: "move", data: 2, id: "same-move" };

test("a committed command survives a lost decision reply, transfer and a retry from a new frame", () => {
  const { join } = fixture({ authority: "host", seats: 2 });
  const alice = join("alice", { score: 0 });
  const bob = join("bob");
  const viewer = join("viewer");
  const { epoch } = alice.opened;
  assert.deepEqual(viewer.opened.seats, ["alice", "bob"]);
  assert.throws(() => viewer.call(command), { status: 403 });
  assert.throws(() => viewer.call({ op: "write", patch: { score: 42 }, sequence: 1 }), { status: 403 });
  assert.throws(() => bob.call({ op: "write", patch: { score: 42 }, sequence: 1, epoch }), { status: 409 });
  assert.deepEqual(bob.call(command), { pending: true });
  assert.deepEqual(alice.read().command, { from: "bob", id: command.id, data: 2, kind: "move" });
  const decision = { op: "decide", from: "bob", id: command.id, epoch, expected: 0,
    patch: { score: 2 }, result: { accepted: 2 }, assignments: { bob: "secret-word" } };
  assert.equal(alice.call(decision).revision, 1);
  assert.equal(alice.call(decision).revision, 1);
  assert.equal(bob.read().assignment, "secret-word");
  assert.ok(!JSON.stringify(viewer.read()).includes("secret-word"));
  assert.throws(() => viewer.call({ op: "checkpoint", epoch }), { status: 409 });
  assert.equal(alice.call({ op: "transfer", epoch, to: "bob" }).host, "bob");
  assert.throws(() => alice.call(decision), { status: 409 });
  assert.throws(() => alice.call({ op: "send", epoch, kind: "host-update", data: 1 }), { status: 409 });
  assert.throws(() => alice.call({ op: "write", epoch, sequence: 1, patch: { score: 99 } }), { status: 409 });
  const returning = join("bob");
  assert.equal(returning.opened.assignment, "secret-word");
  assert.deepEqual(returning.call(command), { revision: 1, result: { accepted: 2 } });
  assert.throws(() => returning.call({ ...command, data: 3 }), { status: 409 });
  assert.deepEqual(returning.call({ op: "checkpoint", epoch: returning.opened.epoch }),
    { revision: 1, state: { score: 2 }, assignments: { bob: "secret-word" } });
  assert.throws(() => bob.call(command), { status: 403 });
  assert.throws(() => bob.call({ op: "open", initialState: {} }), { status: 403 });
  const late = join("late");
  assert.deepEqual(late.opened.changes, [{ revision: 1, state: { score: 2 } }]);
  assert.ok(!JSON.stringify(late.opened).includes("secret-word"));
  returning.call({ op: "seat", claim: false });
  assert.equal(returning.read().assignment, null);
  assert.equal(viewer.call({ op: "seat", claim: true }).assignment, null);
  assert.equal(alice.read().host, "alice");
});

test("uncommitted decisions wait for a connected host and are reassigned after grace", () => {
  const { join, advance, room } = fixture({ authority: "host", limits: { graceSeconds: 2, emptySeconds: 1 } });
  const alice = join("alice");
  const bob = join("bob");
  bob.call(command);
  const previous = alice.read();
  alice.call({ op: "disconnect" });
  assert.equal(bob.read().host, "alice");
  assert.equal(bob.read().participants[0].connected, false);
  advance(2001);
  const next = bob.read();
  assert.equal(next.host, "bob");
  assert.equal(next.command.id, command.id);
  assert.throws(() => alice.call({ op: "decide", epoch: previous.epoch }), { status: 403 });
  bob.call({ op: "decide", epoch: next.epoch, from: "bob", id: command.id, expected: 0, error: "Round closed" });
  assert.deepEqual(bob.call(command), { error: "Round closed" });
  room.revoke("bob");
  assert.throws(() => bob.read(), { status: 403 });
  advance(1001);
  const late = join("late", { fresh: true });
  assert.notEqual(late.opened.sessionId, bob.opened.sessionId);
  assert.deepEqual(late.opened.changes[0].state, { fresh: true });
  room.end();
  assert.throws(() => late.read(), { status: 410 });
  assert.throws(() => join("next"), { status: 410 });
});

test("broadcast replacement is per recipient, directed messages stay private and reconnect discards transient messages", () => {
  const { join } = fixture();
  const alice = join("alice"), bob = join("bob"), carol = join("carol");
  const cursor = { op: "send", kind: "cursor", data: 1, latest: true };
  alice.call(cursor);
  assert.deepEqual(bob.read().messages.map(m => m.data), [1]);
  alice.call({ ...cursor, data: 2 });
  alice.call({ ...cursor, data: 3, to: "bob" });
  alice.call({ op: "send", kind: "private", to: "bob", data: "for-bob" });
  assert.deepEqual(carol.read().messages.map(m => m.data), [2]);
  assert.deepEqual(bob.read().messages.map(m => [m.from, m.data]), [["alice", 3], ["alice", "for-bob"]]);
  assert.deepEqual(alice.read().messages, []);
  alice.call({ ...cursor, data: 4 });
  const replacement = join("bob");
  assert.deepEqual(replacement.read().messages, []);
  assert.deepEqual(join("late").read().messages, []);
  replacement.call({ op: "disconnect" });
  alice.call({ op: "send", kind: "private", to: "bob", data: "offline" });
  assert.throws(() => replacement.read(), { status: 409 });
  assert.deepEqual(join("bob").read().messages, []);
});

test("seat loss refuses an undecided move and invalid private writes cannot partially commit", () => {
  const { join } = fixture({ authority: "host", seats: 2 });
  const alice = join("alice", { score: 0 }), bob = join("bob");
  const submitted = bob.call(command);
  assert.ok(submitted.pending);
  const { epoch, version } = alice.read();
  const action = { op: "decide", epoch, version, expected: 0, from: "bob", id: command.id,
    patch: { score: 1 }, assignments: { bob: 8, "non-member": 1 } };
  assert.throws(() => alice.call(action));
  assert.equal(bob.read().assignment, null);
  assert.deepEqual(alice.read().changes[0], { revision: 0, state: { score: 0 } });
  bob.call({ op: "seat", claim: false });
  assert.ok(bob.call(command).error);
  assert.equal(alice.call(action).revision, 0);
});

test("readers behind history get a snapshot; rate and payload limits are enforced at the authority", () => {
  const { join, advance } = fixture({ limits: { writesPerSecond: 1, messageBytes: 3, maxParticipants: 1 } });
  const alice = join("alice");
  assert.throws(() => join("bob"), { status: 429 });
  assert.throws(() => alice.call({ op: "send", kind: "x", data: "big" }), { status: 413 });
  const write = (seq) => alice.call({ op: "write", sequence: seq, patch: { value: seq } });
  write(1);
  assert.throws(() => write(2), { status: 429 });
  for (let i = 2; i <= 70; i++) { advance(1000); write(i); }
  assert.deepEqual(alice.call({ op: "read", after: 0 }).changes, [{ revision: 70, state: { value: 70 } }]);
});
