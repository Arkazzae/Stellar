const randomUUID = () => globalThis.crypto.randomUUID();

export function fail(status, message) {
  throw Object.assign(new Error(message), { status });
}

export function record(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

const defaults = {
  stateBytes: 256 * 1024, messageBytes: 64 * 1024, writesPerSecond: 32,
  messagesPerSecond: 96, graceSeconds: 10, emptySeconds: 120, maxParticipants: 128,
};
const heartbeatMs = 5000;
const ledgerEntries = 4096;
const ledgerByteLimit = 4 * 1024 * 1024;
const byteSize = value => {
  const text = JSON.stringify(value);
  return typeof Buffer === "undefined" ? globalThis.utf8ByteLength(text) : Buffer.byteLength(text);
};
const validLabel = value => typeof value === "string" && value.length > 0 && value.length <= 128;

function checkObject(value, limit) {
  if (!record(value)) fail(400, "Expected a JSON object.");
  if (byteSize(value) > limit) fail(413, "State is too large.");
}

function acceptRate(timestamps, count, time) {
  while (timestamps.length && timestamps[0] <= time - 1000) timestamps.shift();
  if (timestamps.length >= count) fail(429, "Participant rate limit reached.");
  timestamps.push(time);
}

// The authority receives an authenticated person from its transport. It derives every
// sender, seat and host check from that identity; none is accepted from activity data.
export function createRoom({ authority = "shared", seats, permissions = [], limits: overrides, clock = Date.now } = {}) {
  const limits = { ...defaults, ...overrides };
  const hosted = authority === "host";
  const members = new Map();
  const retired = new Set();
  const assignments = new Map();
  const commands = new Map();
  const pending = new Map();
  let sessionId = randomUUID();
  let host = null;
  let epoch = 0;
  let version = 0;
  let revision = 0;
  let state;
  let history = [];
  let historyBytes = 0;
  let ledgerBytes = 0;
  let emptyAt = null;
  let ended = false;

  function setHost(id) {
    if (host !== id) { host = id; epoch++; version++; }
  }

  function elect() {
    if (hosted && !members.get(host)?.seated)
      setHost([...members.values()].find(member => member.seated && member.connected)?.id ?? null);
  }

  function refusePending(id) {
    for (const [key, command] of pending) {
      if (command.from !== id) continue;
      command.outcome = { error: "This player left their seat." };
      pending.delete(key);
    }
  }

  function remove(member, time) {
    retired.add(member.frame);
    members.delete(member.id);
    assignments.delete(member.id);
    refusePending(member.id);
    version++;
    if (!members.size) emptyAt = time;
  }

  function expire(time) {
    for (const member of members.values()) {
      const offlineAt = member.lastSeen + heartbeatMs;
      if (member.connected && time >= offlineAt) {
        member.connected = false;
        member.offlineAt = offlineAt;
        member.messages = [];
        version++;
      }
      const until = member.offlineAt + limits.graceSeconds * 1000;
      if (!member.connected && time >= until) remove(member, until);
    }
    elect();
    if (emptyAt !== null && time >= emptyAt + limits.emptySeconds * 1000) {
      sessionId = randomUUID();
      state = undefined;
      revision = 0;
      history = [];
      historyBytes = 0;
      commands.clear();
      pending.clear();
      ledgerBytes = 0;
      retired.clear();
      emptyAt = null;
      version++;
    }
  }

  function publicPerson({ id, name, username, avatar, connected }) { return { id, name, username, avatar, connected }; }
  function view(member, time) {
    return {
      version, now: time, host, epoch,
      participants: [...members.values()].map(publicPerson),
      seats: [...members.values()].filter(item => item.seated).map(item => item.id),
      assignment: member.seated ? assignments.get(member.id) ?? null : null,
    };
  }

  function changes(after) {
    if (after === revision) return [];
    if (Number.isInteger(after) && history.length && after >= history[0].revision - 1 && after < revision)
      return history.filter(item => item.revision > after).map(({ state, revision }) => ({ state, revision }));
    return [{ state, revision }];
  }
  function commit(next) {
    state = next;
    revision++;
    const bytes = byteSize(next);
    history.push({ revision, state, bytes });
    historyBytes += bytes;
    while (history.length > 64 || historyBytes > 1048576) historyBytes -= history.shift().bytes;
  }
  function patched(patch) {
    checkObject(patch, limits.stateBytes);
    const next = { ...state, ...patch };
    checkObject(next, limits.stateBytes);
    return next;
  }
  function requireSeat(member) {
    if (!member.seated) fail(403, "A playing seat is required.");
  }
  function requireHost(member, input) {
    if (!hosted || host !== member.id || epoch !== input.epoch) fail(409, "The host changed.");
  }

  function open(person, input, time) {
    checkObject(input.initialState, limits.stateBytes);
    if (retired.has(input.frame)) fail(403, "This frame is no longer active.");
    if (retired.size >= 16384) fail(429, "Session frame limit reached.");
    let member = members.get(person.id);
    if (!member) {
      if (members.size >= limits.maxParticipants) fail(429, "The session is full.");
      const occupied = [...members.values()].filter(item => item.seated).length;
      member = { id: person.id, seated: seats == null || occupied < seats, writes: [], sends: [] };
      members.set(person.id, member);
    }
    if (member.frame !== input.frame) {
      if (member.frame) retired.add(member.frame);
      Object.assign(member, { frame: input.frame, sequence: 0, committedRevision: 0, messages: [] });
    }
    Object.assign(member, { name: person.name, username: person.username, avatar: person.avatar ?? null, connected: true, suspended: false, lastSeen: time });
    emptyAt = null;
    state ??= input.initialState;
    version++;
    elect();
    return { sessionId, me: publicPerson(member), authority, permissions, limits,
      changes: [{ revision, state }], ...view(member, time) };
  }

  function message(member, input, time) {
    if (!validLabel(input.kind) || input.data === undefined ||
        (input.to !== undefined && !validLabel(input.to))) fail(400, "Invalid message.");
    if (input.binary) {
      if (typeof input.data !== "string" || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(input.data))
        fail(400, "Invalid binary message.");
    }
    const size = input.binary ? input.data.length / 4 * 3 - (input.data.endsWith("==") ? 2 : Number(input.data.endsWith("="))) : byteSize(input.data);
    if (size > limits.messageBytes) fail(413, "Message is too large.");
    if (input.epoch !== undefined) requireHost(member, input);
    acceptRate(member.sends, limits.messagesPerSecond, time);
    const event = { from: member.id, kind: input.kind, data: input.data, binary: !!input.binary, latest: !!input.latest };
    for (const recipient of members.values()) {
      if (!recipient.connected || (input.to === undefined ? recipient === member : recipient.id !== input.to)) continue;
      if (input.latest) recipient.messages = recipient.messages.filter(old =>
        !(old.latest && old.from === event.from && old.kind === event.kind));
      if (recipient.messages.length >= 32) recipient.messages.shift();
      recipient.messages.push(event);
    }
    return { accepted: true };
  }

  function commandKey(person, id) { return JSON.stringify([person, id]); }
  function submit(member, input, time) {
    if (!hosted) fail(400, "Commands require host authority.");
    if (!validLabel(input.id) || !validLabel(input.kind) || input.data === undefined) fail(400, "Invalid command.");
    const dataBytes = byteSize(input.data);
    if (dataBytes > limits.messageBytes) fail(413, "Command is too large.");
    const key = commandKey(member.id, input.id);
    const known = commands.get(key);
    if (known) {
      if (known.kind !== input.kind || JSON.stringify(known.data) !== JSON.stringify(input.data))
        fail(409, "This command ID belongs to a different request.");
      return known.outcome ?? { pending: true };
    }
    requireSeat(member);
    if (commands.size >= ledgerEntries || pending.size >= 128 || ledgerBytes + dataBytes > ledgerByteLimit)
      fail(429, "Command capacity reached.");
    acceptRate(member.sends, limits.messagesPerSecond, time);
    const command = { from: member.id, id: input.id, kind: input.kind, data: input.data };
    ledgerBytes += dataBytes;
    commands.set(key, command);
    pending.set(key, command);
    return { pending: true };
  }

  function decide(member, input, time) {
    requireHost(member, input);
    const key = commandKey(input.from, input.id);
    const command = commands.get(key);
    if (!command) fail(404, "Unknown command.");
    if (command.outcome) return { revision, changes: changes(input.expected) };
    if (input.expected !== revision || (input.version !== undefined && input.version !== version))
      return { conflict: true, changes: changes(input.expected) };
    if (input.error !== undefined) {
      if (typeof input.error !== "string" || input.error.length > 512)
        fail(400, "Invalid command refusal.");
      command.outcome = { error: input.error || "Command refused." };
    } else {
      const resultBytes = input.result === undefined ? 0 : byteSize(input.result);
      if (resultBytes > limits.messageBytes || ledgerBytes + resultBytes > ledgerByteLimit) fail(413, "Result is too large.");
      const next = input.patch === undefined ? state : patched(input.patch);
      if (input.assignments !== undefined) {
        checkObject(input.assignments, limits.stateBytes);
        const checkpoint = new Map(assignments);
        for (const [id, value] of Object.entries(input.assignments)) {
          if (!members.get(id)?.seated) fail(400, "Assignment recipient has no seat.");
          if (value === null) checkpoint.delete(id);
          else checkpoint.set(id, value);
        }
        checkObject(Object.fromEntries(checkpoint), limits.stateBytes);
        acceptRate(member.writes, limits.writesPerSecond, time);
        assignments.clear();
        for (const [id, value] of checkpoint) assignments.set(id, value);
        version++;
      } else {
        acceptRate(member.writes, limits.writesPerSecond, time);
      }
      commit(next);
      ledgerBytes += resultBytes;
      command.outcome = { result: input.result, revision };
    }
    pending.delete(key);
    return { revision, changes: changes(input.expected) };
  }

  function room(person, input) {
    if (ended) fail(410, "The session ended.");
    if (!record(input) || typeof input.frame !== "string" || input.frame.length !== 36) fail(400, "Invalid session request.");
    const time = clock();
    expire(time);
    if (input.op === "open") return open(person, input, time);
    const member = members.get(person.id);
    if (member?.frame !== input.frame) fail(403, "This frame is no longer active.");
    if (!member.connected && input.op !== "disconnect" && input.op !== "close") {
      if (member.suspended || input.op !== "read") fail(409, "The frame is disconnected.");
      member.connected = true;
      version++;
      elect();
    }
    if (typeof person.name === "string") updateProfile(person);
    member.lastSeen = time;
    switch (input.op) {
      case "read": {
        const response = { ...view(member, time), changes: changes(input.after), messages: member.messages.splice(0) };
        if (host === person.id && pending.size) {
          response.command = pending.values().next().value;
          response.commandRevision = revision;
          response.checkpoint = Object.fromEntries(assignments);
        }
        return response;
      }
      // The transport owns private storage. Validate the frame and payload here first.
      case "loadProgress": return {};
      case "saveProgress":
        checkObject(input.data, 64 * 1024);
        if (!Number.isSafeInteger(input.expected) || input.expected < 0 || input.expected >= Number.MAX_SAFE_INTEGER)
          fail(400, "Expected a progress revision.");
        acceptRate(member.writes, limits.writesPerSecond, time);
        return {};
      case "sync": return { ...view(member, time), changes: changes(input.after) };
      case "write": {
        requireSeat(member);
        if (hosted) requireHost(member, input);
        if (!Number.isSafeInteger(input.sequence) || input.sequence < 1 || input.sequence < member.sequence)
          fail(400, "Invalid write sequence.");
        if (input.sequence === member.sequence) return { revision: member.committedRevision, changes: changes(input.after) };
        if (input.expected !== undefined && input.expected !== revision) return { conflict: true, changes: changes(input.after) };
        const next = patched(input.patch);
        acceptRate(member.writes, limits.writesPerSecond, time);
        commit(next);
        member.sequence = input.sequence;
        member.committedRevision = revision;
        return { revision, changes: changes(input.after) };
      }
      case "send": return message(member, input, time);
      case "submit": return submit(member, input, time);
      case "decide": return decide(member, input, time);
      case "transfer":
        requireHost(member, input);
        if (!members.get(input.to)?.connected || !members.get(input.to).seated) fail(409, "Choose a connected player.");
        setHost(input.to);
        return view(member, time);
      case "checkpoint":
        requireHost(member, input);
        return { state, revision, assignments: Object.fromEntries(assignments) };
      case "seat": {
        if (typeof input.claim !== "boolean") fail(400, "Invalid seat request.");
        if (input.claim && !member.seated && seats != null && [...members.values()].filter(item => item.seated).length >= seats)
          fail(409, "All playing seats are taken.");
        if (member.seated !== input.claim) {
          member.seated = input.claim;
          if (!member.seated) { assignments.delete(member.id); refusePending(member.id); }
          version++;
          elect();
        }
        return view(member, time);
      }
      case "disconnect":
        member.suspended = true;
        if (member.connected) {
          member.connected = false;
          member.offlineAt = time;
          member.messages = [];
          version++;
        }
        return { disconnected: true };
      case "close":
        remove(member, time);
        elect();
        return { closed: true };
      default: fail(400, "Unknown session operation.");
    }
  }
  function updateProfile({ id, name, username, avatar = null }) {
    const member = members.get(id);
    if (member && (member.name !== name || member.username !== username || member.avatar !== avatar)) {
      Object.assign(member, { name, username, avatar });
      version++;
    }
  }
  room.updateProfile = updateProfile;
  const originalSessionId = sessionId;
  room.expired = () => {
    expire(clock());
    return sessionId !== originalSessionId;
  };
  room.revoke = id => {
    const member = members.get(id);
    if (member) { remove(member, clock()); elect(); }
  };
  room.end = () => { ended = true; members.clear(); commands.clear(); pending.clear(); assignments.clear(); };
  return room;
}
