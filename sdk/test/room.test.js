import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import test from "node:test";
import { createRoom } from "../dev/room.js";

test("concurrent joins, a conflict, a lost reply and a returning voter", () => {
  const room = createRoom();
  const connect = (id, initialState) => {
    const frame = randomUUID();
    const call = input => room({ id, name: id }, { ...input, frame });
    return { call, ...call({ op: "open", initialState }) };
  };
  const a = connect("alice", {});
  const b = connect("bob", { ignored: true });
  assert.equal(a.sessionId, b.sessionId);
  assert.deepEqual(b.changes[0].state, {});
  const write = { op: "write", after: 0, sequence: 1, expected: 0, patch: { alice: "yes" } };
  assert.equal(a.call(write).revision, 1);
  assert.equal(a.call(write).revision, 1);
  assert.equal(b.call({ ...write, patch: { bob: "no" } }).conflict, true);
  b.call({ ...write, expected: 1, patch: { bob: "no" } });
  const back = connect("alice", {});
  assert.deepEqual(back.changes[0].state, { alice: "yes", bob: "no" });
  assert.throws(() => a.call(write), { status: 403 });
  back.call({ ...write, expected: 2, patch: { alice: "no" } });
  const updates = b.call({ op: "read", after: 0 }).changes;
  assert.deepEqual(updates.map(item => item.revision), [1, 2, 3]);
  assert.deepEqual(updates.at(-1).state, { alice: "no", bob: "no" });
});
