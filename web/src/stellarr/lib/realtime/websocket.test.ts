import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { forgetActiveServerId, rememberActiveServerId } from "@/lib/api/transport";
import { RTC_PROTOCOL_VERSION } from "@/lib/rtc/protocol";
import { WebSocketRealtimeClient } from "./websocket";

class FakeSocket {
  bufferedAmount = 0;
  protocol = "stellarr.v1";
  readyState: WebSocket["readyState"] = WebSocket.CONNECTING;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  send = vi.fn<(data: string) => void>();
  close = vi.fn(() => { this.readyState = WebSocket.CLOSED; });

  open(): void {
    this.readyState = WebSocket.OPEN;
  }

  ready(): void {
    this.open();
    this.receive({ type: "ready" });
  }

  receive(frame: unknown): void {
    this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(frame) }));
  }

  event(name: string, data: unknown): void {
    this.receive({ type: "event", name, data });
  }

  drop(code = 1006): void {
    this.readyState = WebSocket.CLOSED;
    this.onclose?.(new CloseEvent("close", { code }));
  }

  sentFrames(): unknown[] {
    return this.send.mock.calls.map(([frame]) => JSON.parse(frame) as unknown);
  }
}

function createClient() {
  const sockets: FakeSocket[] = [];
  const connect = vi.fn((_url: string, _protocol: string) => {
    const socket = new FakeSocket();
    sockets.push(socket);
    return socket;
  });
  const client = new WebSocketRealtimeClient(connect);
  const socket = () => sockets.at(-1)!;
  return {
    client,
    connect,
    socket,
    async open() {
      const started = client.start();
      socket().ready();
      await started;
      return socket();
    },
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(Math, "random").mockReturnValue(0.5);
  forgetActiveServerId();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("realtime WebSocket", () => {
  it("requests stellarr.v1 and waits for the ready message before sending commands", async () => {
    const { client, connect, socket } = createClient();
    const status: string[] = [];
    client.subscribeStatus(() => status.push(client.getStatus()));
    const started = client.start();
    const duplicate = client.start();
    socket().open();

    expect(connect).toHaveBeenCalledExactlyOnceWith(
      `ws://${window.location.host}/hubs/app`, "stellarr.v1",
    );
    await expect(client.joinVoice("voice-1")).rejects.toThrow("realtime connection is not open");
    expect(status).toEqual(["connecting"]);
    socket().receive({ type: "ready" });
    await Promise.all([started, duplicate]);
    await client.stop();
    expect(status).toEqual(["connecting", "connected", "idle"]);
  });

  it("keeps activity status codes and rejects an oversized request before sending", async () => {
    const { client, open, socket } = createClient();
    await open();
    const body = { install: "game", access: "lease", action: { op: "read" } };
    const read = client.activityRoom("personal", body);
    expect(socket().sentFrames()).toEqual([{ type: "call", id: 1, method: "ActivityRoom", args: ["personal", body] }]);
    socket().receive({ type: "result", id: 1, value: { value: { revision: 4 } } });
    await expect(read).resolves.toEqual({ revision: 4 });
    const refused = client.activityRoom("personal", body);
    socket().receive({ type: "result", id: 2, value: { error: "The host changed.", status: 409 } });
    await expect(refused).rejects.toMatchObject({ status: 409 });
    await expect(client.activityRoom("personal", { action: { op: "write", patch: { board: "x".repeat(200_000) } } }))
      .rejects.toMatchObject({ code: "frame_too_large" });
    expect(socket().sentFrames()).toHaveLength(2);
  });

  it("adds the active server to the hub query and preserves other parameters", async () => {
    rememberActiveServerId("server 1");
    const { client, connect, open } = createClient();
    client.configureRtc({ signalingHub: "/hubs/rtc?v=2&serverId=stale" });

    await open();

    const url = new URL(connect.mock.calls[0]![0]);
    expect(url.pathname).toBe("/hubs/rtc");
    expect(url.searchParams.get("v")).toBe("2");
    expect(url.searchParams.getAll("serverId")).toEqual(["server 1"]);
  });

  it.each(["//evil.example/hub", "/\\evil.example/hub", "/\t/evil.example/hub", "https://evil.example/hub"])(
    "ignores a cross-origin bootstrap path (%s)",
    async (signalingHub) => {
      const { client, connect, open } = createClient();
      client.configureRtc({ signalingHub });
      await open();
      expect(connect.mock.calls[0]?.[0]).toBe(`ws://${window.location.host}/hubs/app`);
    },
  );

  it("retries failed upgrades with jitter and recovers without callers starting it again", async () => {
    const { client, connect, socket } = createClient();
    const starting = expect(client.start()).rejects.toThrow("1006");
    socket().drop();
    await starting;
    expect(client.getStatus()).toBe("disconnected");

    await vi.advanceTimersByTimeAsync(1_124);
    expect(connect).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(connect).toHaveBeenCalledTimes(2);
    socket().drop();
    await vi.advanceTimersByTimeAsync(2_249);
    expect(connect).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    socket().ready();

    expect(client.getStatus()).toBe("connected");
  });

  it("closes and retries a socket that never becomes ready", async () => {
    const { client, connect, socket } = createClient();
    const starting = expect(client.start()).rejects.toThrow("waiting for realtime readiness");
    const first = socket();
    first.open();
    await vi.advanceTimersByTimeAsync(20_000);
    await starting;
    expect(first.close).toHaveBeenCalledOnce();

    await vi.advanceTimersByTimeAsync(1_125);
    expect(connect).toHaveBeenCalledTimes(2);
    socket().ready();
    expect(client.getStatus()).toBe("connected");
  });

  it("keeps a quiet socket with heartbeats and reconnects after server silence", async () => {
    const { client, open, socket } = createClient();
    const first = await open();
    await vi.advanceTimersByTimeAsync(30_000);
    first.receive({ type: "ping" });
    await vi.advanceTimersByTimeAsync(44_999);
    expect(client.getStatus()).toBe("connected");

    const inFlight = expect(client.joinChannel("c1")).rejects.toThrow("waiting for realtime traffic");
    await vi.advanceTimersByTimeAsync(1);
    await inFlight;
    expect(client.getStatus()).toBe("reconnecting");
    expect(first.close).toHaveBeenCalledOnce();

    await vi.advanceTimersByTimeAsync(1_125);
    const second = socket();
    second.ready();
    expect(second.send).not.toHaveBeenCalled();
    expect(client.getStatus()).toBe("connected");
  });

  it.each([1011, 1012, 1013, 1006])("reconnects after transient close %s without replaying a call", async (code) => {
    const { client, open, socket } = createClient();
    const first = await open();
    const inFlight = expect(client.joinChannel("c1")).rejects.toThrow(`closed (${code})`);
    first.drop(code);
    await inFlight;
    expect(client.getStatus()).toBe("reconnecting");

    await vi.advanceTimersByTimeAsync(1_125);
    const second = socket();
    second.ready();

    expect(second).not.toBe(first);
    expect(second.send).not.toHaveBeenCalled();
    expect(client.getStatus()).toBe("connected");
  });

  it.each([1000, 1002, 1007, 1009, 4401, 4406])("ends retries after terminal close %s", async (code) => {
    const { client, connect, open } = createClient();
    const first = await open();
    first.drop(code);

    expect(client.getStatus()).toBe("disconnected");
    expect(client.hasSessionEnded()).toBe(code === 4401);
    await expect(client.start()).rejects.toThrow(`closed (${code})`);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(connect).toHaveBeenCalledTimes(1);
  });

  it("reports an explicit session end even during retries with an unchanged status", async () => {
    const { client, socket } = createClient();
    const changed = vi.fn(() => client.hasSessionEnded());
    client.subscribeStatus(changed);
    const starting = expect(client.start()).rejects.toThrow("1006");
    socket().drop();
    await starting;
    await vi.advanceTimersByTimeAsync(1_125);
    socket().drop(4401);

    expect(changed.mock.results.map(({ value }) => value)).toEqual([false, false, true]);
  });

  it.each([
    [{ type: "event", name: "TypingChanged", data: {} }, false],
    [{ type: "ready" }, true],
    [{ type: "result", id: 0, value: 0 }, true],
    [{ type: "result", id: 1 }, true],
    [{ type: "error", id: 1, code: 42 }, true],
    [[{ type: "result", id: 1, value: 0 }], true],
  ])("terminates on malformed or out-of-order protocol frames (%j)", async (frame, ready) => {
    const { client, open, socket, connect } = createClient();
    if (ready) await open();
    else void client.start().catch(() => undefined);
    socket().receive(frame);
    expect(client.getStatus()).toBe("disconnected");
    expect(socket().close).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(connect).toHaveBeenCalledTimes(1);
  });

  it("invalidates old callbacks on server switch and preserves subscriptions", async () => {
    const { client, connect, socket } = createClient();
    const changed = vi.fn();
    client.on("SocialChanged", changed);
    rememberActiveServerId("first");
    const abandoned = expect(client.start()).rejects.toMatchObject({ name: "AbortError" });
    const first = socket();
    const oldClose = first.onclose;
    const oldMessage = first.onmessage;

    rememberActiveServerId("second");
    const starting = client.start();
    await abandoned;
    expect(first.close).toHaveBeenCalledExactlyOnceWith(1000);
    oldMessage?.(new MessageEvent("message", { data: '{"type":"ready"}' }));
    oldClose?.(new CloseEvent("close", { code: 4401 }));
    expect(client.getStatus()).toBe("connecting");
    expect(client.hasSessionEnded()).toBe(false);
    socket().ready();
    await starting;
    socket().event("SocialChanged", {});
    expect(changed).toHaveBeenCalledOnce();
    expect(connect.mock.calls[1]?.[0]).toContain("serverId=second");
  });

  it("cancels a retry when the client stops", async () => {
    const { client, connect, open } = createClient();
    const first = await open();
    first.drop();

    await client.stop();
    await vi.advanceTimersByTimeAsync(60_000);

    expect(client.getStatus()).toBe("idle");
    expect(connect).toHaveBeenCalledTimes(1);
  });

  it("drops incorrectly shaped payloads and tolerates new event names", async () => {
    const { client, open } = createClient();
    const changed = vi.fn();
    client.on("NotificationInboxChanged", changed);
    const socket = await open();

    socket.event("NotificationInboxChanged", { serverId: "server-1", sequence: 12 });
    socket.event("NotificationInboxChanged", { serverId: "server-2", global: true });
    socket.event("NotificationInboxChanged", { serverId: "server-1", sequence: "12" });
    socket.event("NotificationInboxChanged", { sequence: 13 });
    socket.event("FutureEvent", {});
    socket.event("toString", {});

    expect(changed.mock.calls).toEqual([
      [{ serverId: "server-1", sequence: 12 }],
      [{ serverId: "server-2", global: true }],
    ]);
    expect(client.getStatus()).toBe("connected");
  });

  it("filters stale voice occupancy per room and resets sequences after a reconnect", async () => {
    const { client, open, socket } = createClient();
    const changed = vi.fn();
    client.on("VoiceParticipantChanged", changed);
    const first = await open();
    const occupancy = (connection: FakeSocket, sequence: number, channelId = "v1") =>
      connection.event("VoiceParticipantChanged", {
        channelId, sequence, participants: [{ userId: "u1", displayName: "Ada" }],
      });

    occupancy(first, 8);
    occupancy(first, 8);
    occupancy(first, 7);
    occupancy(first, 2, "v2");
    first.drop();
    await vi.advanceTimersByTimeAsync(1_125);
    const second = socket();
    second.ready();
    occupancy(second, 1);

    expect(changed.mock.calls.map(([event]) => event.sequence)).toEqual([8, 2, 1]);
  });
});

describe("realtime commands", () => {
  it("correlates out-of-order results and does not allocate a response for typing", async () => {
    const { client, open } = createClient();
    const socket = await open();
    const clock = client.getRtcClock();
    const channel = client.joinChannel("c1");
    await client.sendTyping("c1");

    expect(socket.sentFrames()).toEqual([
      { type: "call", id: 1, method: "GetRtcClock", args: [] },
      { type: "call", id: 2, method: "JoinChannel", args: ["c1"] },
      { type: "send", method: "SendTyping", args: ["c1"] },
    ]);

    socket.receive({ type: "result", id: 2, value: null });
    socket.receive({ type: "result", id: 1, value: 1_800_000_000_000 });

    await expect(channel).resolves.toBeUndefined();
    await expect(clock).resolves.toBe(1_800_000_000_000);
  });

  it("rejects a malformed RTC clock result", async () => {
    const { client, open } = createClient();
    const socket = await open();
    const answer = expect(client.getRtcClock()).rejects.toThrow("no clock value");
    socket.receive({ type: "result", id: 1, value: "1800000000" });
    await answer;
  });

  it("passes on a hub refusal with its code", async () => {
    const { client, open } = createClient();
    const socket = await open();
    const answer = expect(client.openMediaSession("voice-1")).rejects.toMatchObject({
      code: "visible",
      message: "Media server unavailable",
    });
    socket.receive({ type: "error", id: 1, code: "visible", message: "Media server unavailable" });
    await answer;
  });

  it("requires a valid media session id and sends media signals unchanged", async () => {
    const { client, open } = createClient();
    const socket = await open();
    const invalid = expect(client.openMediaSession("voice-1")).rejects.toThrow("no session id");
    socket.receive({ type: "result", id: 1, value: { id: "session-7" } });
    await invalid;

    const opened = expect(client.openMediaSession("voice-1")).resolves.toBe("session-7");
    socket.receive({ type: "result", id: 2, value: { sessionId: "session-7" } });
    await opened;

    const signal = {
      protocolVersion: RTC_PROTOCOL_VERSION,
      channelId: "voice-1",
      sessionId: "session-7",
      kind: "candidate" as const,
      payload: "null",
    };
    const sent = client.sendMediaSignal(signal);
    socket.receive({ type: "result", id: 3, value: null });
    await sent;

    expect(socket.sentFrames()).toEqual([
      { type: "call", id: 1, method: "OpenMediaSession", args: ["voice-1"] },
      { type: "call", id: 2, method: "OpenMediaSession", args: ["voice-1"] },
      { type: "call", id: 3, method: "SendMediaSignal", args: [signal] },
    ]);
  });

  it("relays only valid signals and closures from the media server", async () => {
    const { client, open } = createClient();
    const signals = vi.fn();
    const closures = vi.fn();
    client.on("MediaSignal", signals);
    client.on("MediaSessionClosed", closures);
    const socket = await open();
    const signal = {
      protocolVersion: RTC_PROTOCOL_VERSION,
      channelId: "voice-1",
      sessionId: "session-7",
      kind: "answer",
      payload: '{"type":"answer","sdp":"v=0"}',
    };
    const closure = { channelId: "voice-1", sessionId: "session-7", reason: "media-restarted" };

    socket.event("MediaSignal", signal);
    socket.event("MediaSignal", { ...signal, kind: "candidate" });
    socket.event("MediaSignal", { ...signal, protocolVersion: 2 });
    socket.event("MediaSessionClosed", closure);
    socket.event("MediaSessionClosed", { ...closure, reason: undefined });

    expect(signals.mock.calls).toEqual([[signal]]);
    expect(closures.mock.calls).toEqual([[closure]]);
  });

  it("bounds the UTF-8 frame size and transport send buffer", async () => {
    const { client, open } = createClient();
    const socket = await open();

    await expect(client.joinChannel("ż".repeat(81_920))).rejects.toThrow("message is too large");
    socket.bufferedAmount = 1024 * 1024;
    await expect(client.sendTyping("c1")).rejects.toThrow("send buffer is full");
    expect(socket.send).not.toHaveBeenCalled();
    expect(client.getStatus()).toBe("connected");
  });

  it("expires pending calls, ignores late results and bounds the pending queue", async () => {
    const { client, open } = createClient();
    const socket = await open();
    const calls = Array.from({ length: 128 }, () =>
      expect(client.getRtcClock()).rejects.toThrow("timed out"),
    );
    await expect(client.getRtcClock()).rejects.toThrow("Too many realtime calls");
    await vi.advanceTimersByTimeAsync(15_000);
    await Promise.all(calls);

    const clock = expect(client.getRtcClock()).resolves.toBe(400);
    socket.receive({ type: "result", id: 1, value: 100 });
    socket.receive({ type: "result", id: 129, value: 400 });
    await clock;
  });
});
