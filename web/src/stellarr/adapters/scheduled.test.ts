import { describe, expect, it, vi } from "vitest";

import { parseScheduledMessage, type ScheduledApiClient } from "@/lib/api/calls/scheduled";
import { ApiError } from "@/lib/api/errors";
import type { RealtimeClient, RealtimeStatus } from "@/lib/realtime/types";
import { ScheduleRefusedError, type Message, type ScheduledMessage } from "@/ports";

import { createScheduledAdapter } from "./scheduled";

/** The message as the server writes it: every missing value a null. */
function wire(id: string, sendAt: string, extra: Record<string, unknown> = {}) {
  return {
    id,
    target: { kind: "channel", serverId: "s1", serverName: "Klub", channelId: "c1", channelName: "general" },
    content: `text ${id}`,
    attachments: [],
    replyToMessageId: null,
    sendAt,
    createdAt: "2026-11-26T10:00:00Z",
    state: "pending",
    failure: null,
    ...extra,
  };
}
const item = (id: string, sendAt: string): ScheduledMessage => parseScheduledMessage(wire(id, sendAt), "test");

function fakeRealtime() {
  const handlers = new Map<string, (payload: unknown) => void>();
  const statusListeners = new Set<() => void>();
  let status: RealtimeStatus = "connected";
  const realtime = {
    on: (event: string, handler: (payload: unknown) => void) => {
      handlers.set(event, handler);
      return () => handlers.delete(event);
    },
    getStatus: () => status,
    subscribeStatus: (listener: () => void) => {
      statusListeners.add(listener);
      return () => statusListeners.delete(listener);
    },
  } as unknown as RealtimeClient;
  return {
    realtime,
    emit: (event: string, payload: unknown) => handlers.get(event)?.(payload),
    setStatus(next: RealtimeStatus) {
      status = next;
      for (const listener of statusListeners) listener();
    },
  };
}

function setup(first: ScheduledMessage[] = []) {
  const api = {
    listScheduled: vi.fn<ScheduledApiClient["listScheduled"]>(async () => first),
    createScheduled: vi.fn<ScheduledApiClient["createScheduled"]>(async (_target, body) => item("new", body.sendAt)),
    updateScheduled: vi.fn<ScheduledApiClient["updateScheduled"]>(async (id, patch) =>
      item(id, patch.sendAt ?? "2026-11-28T08:00:00Z"),
    ),
    deleteScheduled: vi.fn<ScheduledApiClient["deleteScheduled"]>(async () => {}),
    sendScheduled: vi.fn<ScheduledApiClient["sendScheduled"]>(async () => ({ id: "a" }) as Message),
  };
  const hub = fakeRealtime();
  const port = createScheduledAdapter(api, hub.realtime);
  const ids = () => port.getSnapshot().map((message) => message.id);
  return { api, hub, port, ids };
}

const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("the scheduled messages port", () => {
  it("reads the list once, when it is first looked at, soonest first", async () => {
    const { api, port, ids } = setup([item("b", "2026-11-28T08:00:00Z"), item("a", "2026-11-27T08:00:00Z")]);
    expect(api.listScheduled).not.toHaveBeenCalled();

    port.subscribe(() => {});
    port.subscribe(() => {});
    await settled();

    expect(api.listScheduled).toHaveBeenCalledTimes(1);
    expect(ids()).toEqual(["a", "b"]);
  });

  it("follows the author's live events, and keeps one that came while the list was being read", async () => {
    const { api, hub, port, ids } = setup();
    let answer: (list: ScheduledMessage[]) => void = () => {};
    api.listScheduled.mockImplementation(() => new Promise((resolve) => (answer = resolve)));
    port.subscribe(() => {});

    hub.emit("ScheduledMessageChanged", wire("live", "2026-11-29T08:00:00Z"));
    answer([item("old", "2026-11-27T08:00:00Z")]);
    await settled();
    expect(ids()).toEqual(["old", "live"]);

    hub.emit("ScheduledMessageRemoved", { id: "old", sent: true });
    expect(ids()).toEqual(["live"]);
  });

  it("reads the list again after a reconnect, since events may have been missed", async () => {
    const { api, hub, port } = setup();
    port.subscribe(() => {});
    await settled();
    hub.setStatus("reconnecting");
    hub.setStatus("connected");
    await settled();
    expect(api.listScheduled).toHaveBeenCalledTimes(2);
  });

  it("schedules with the time in UTC and a reply only for a channel, and lists the answer at once", async () => {
    const { api, port, ids } = setup();
    await port.schedule(
      { kind: "channel", channelId: "c1" },
      { content: "hej", attachmentIds: ["f1"], replyToMessageId: "root", sendAt: new Date("2026-11-27T08:00:00Z") },
    );
    expect(api.createScheduled).toHaveBeenLastCalledWith(
      { kind: "channel", channelId: "c1" },
      { content: "hej", attachmentIds: ["f1"], replyToMessageId: "root", sendAt: "2026-11-27T08:00:00.000Z" },
    );
    expect(ids()).toEqual(["new"]);

    await port.schedule(
      { kind: "conversation", conversationId: "d1" },
      { content: "hej", attachmentIds: [], replyToMessageId: "x", sendAt: new Date("2026-11-27T08:00:00Z") },
    );
    expect(api.createScheduled).toHaveBeenLastCalledWith(
      { kind: "conversation", conversationId: "d1" },
      { content: "hej", attachmentIds: [], sendAt: "2026-11-27T08:00:00.000Z" },
    );
  });

  it("drops a message once it was sent now or deleted", async () => {
    const { api, port, ids } = setup([item("a", "2026-11-27T08:00:00Z"), item("b", "2026-11-28T08:00:00Z")]);
    port.subscribe(() => {});
    await settled();

    await port.sendNow("a");
    await port.remove("b");

    expect(api.sendScheduled).toHaveBeenCalledWith("a");
    expect(api.deleteScheduled).toHaveBeenCalledWith("b");
    expect(ids()).toEqual([]);
  });

  it.each([
    [new ApiError("invalid", "x", 409, { code: "scheduled-limit" }), "limit"],
    [new ApiError("invalid", "x", 409, { code: "scheduled-encrypted" }), "encrypted"],
    [new ApiError("invalid", "x", 409, { code: "scheduled-failed" }), "failed"],
    [new ApiError("invalid", "x", 400, { fields: { sendAt: "Too late." } }), "time"],
    [new ApiError("invalid", "x", 400, { fields: { attachmentIds: "Too many." } }), "attachments"],
    [new ApiError("forbidden", "x", 403), "forbidden"],
    [new ApiError("rate-limited", "x", 429), "slowMode"],
    [new Error("offline"), "other"],
  ])("says why a call was refused: %s", async (refusal, reason) => {
    const { api, port } = setup();
    api.createScheduled.mockRejectedValue(refusal);
    const attempt = port.schedule(
      { kind: "channel", channelId: "c1" },
      { content: "hej", attachmentIds: [], sendAt: new Date("2026-11-27T08:00:00Z") },
    );
    await expect(attempt).rejects.toBeInstanceOf(ScheduleRefusedError);
    await expect(attempt).rejects.toMatchObject({ reason });
  });

  it("reads the list again when sending now finds the message changed at that moment", async () => {
    const { api, port } = setup([item("a", "2026-11-27T08:00:00Z")]);
    port.subscribe(() => {});
    await settled();
    api.sendScheduled.mockRejectedValue(new ApiError("invalid", "x", 409, { details: { empty: true } }));

    await expect(port.sendNow("a")).rejects.toMatchObject({ reason: "changed" });
    expect(api.listScheduled).toHaveBeenCalledTimes(2);
  });
});

describe("a scheduled message from the server", () => {
  it("leaves out what the server sends as null, and reads an unknown failure as other", () => {
    const pending = parseScheduledMessage(wire("a", "2026-11-27T08:00:00Z"), "test");
    expect(pending).not.toHaveProperty("replyToMessageId");
    expect(pending).not.toHaveProperty("failure");
    expect(pending.target).toEqual({
      kind: "channel",
      serverId: "s1",
      serverName: "Klub",
      channelId: "c1",
      channelName: "general",
    });

    // The names are null once the channel or its server is gone.
    const gone = parseScheduledMessage(
      wire("g", "2026-11-27T08:00:00Z", {
        target: { kind: "channel", serverId: "s1", serverName: null, channelId: "c1", channelName: null },
      }),
      "test",
    );
    expect(gone.target).toEqual({ kind: "channel", serverId: "s1", channelId: "c1" });

    const failed = parseScheduledMessage(
      wire("b", "2026-11-24T07:00:00Z", { state: "failed", failure: { code: "something_new", at: "2026-11-24T07:00:01Z" } }),
      "test",
    );
    expect(failed.failure).toEqual({ code: "other", at: "2026-11-24T07:00:01Z" });
  });
});
