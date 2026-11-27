import { act, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import type { ScheduledMessage, ScheduledTarget } from "@/ports";
import { answerConfirm } from "@/test/confirm";
import { polish } from "@/test/translators";

import { useScheduledListActions, useWaitingHere, type ScheduledPlace } from "./useScheduled";

const port = vi.hoisted(() => ({
  items: [] as ScheduledMessage[],
  sendNow: vi.fn(async () => ({})),
  edit: vi.fn(async () => {}),
  remove: vi.fn(async () => {}),
}));

vi.mock("@/state/hooks", () => ({
  usePorts: () => ({
    scheduled: {
      subscribe: () => () => {},
      getSnapshot: () => port.items,
      sendNow: port.sendNow,
      edit: port.edit,
      remove: port.remove,
    },
  }),
  useDirectory: () => ({ textChannels: [{ id: "c1", name: "general" }] }),
  useFriends: () => ({ conversations: [{ id: "d1", peer: { id: "u-ania", displayName: "Ania Mazur" } }] }),
  useSession: () => ({ user: { id: "me", displayName: "Maja" } }),
  useLanguage: () => ({ locale: "pl", t: polish }),
}));

const CHANNEL: ScheduledTarget = { kind: "channel", channelId: "c1" };
const item = (
  id: string,
  sendAt: string,
  target: ScheduledTarget,
  extra: Partial<ScheduledMessage> = {},
): ScheduledMessage => ({
  id,
  target,
  content: `text ${id}`,
  attachments: [],
  sendAt,
  createdAt: "2026-11-25T10:00:00Z",
  state: "pending",
  ...extra,
});

/** Soonest first, as the port keeps them. */
const ITEMS = [
  item("failed", "2026-11-24T07:00:00Z", CHANNEL, {
    state: "failed",
    failure: { code: "other", at: "2026-11-24T07:00:01Z" },
  }),
  item("channel", "2026-11-27T08:00:00Z", CHANNEL),
  item("thread", "2026-11-28T08:00:00Z", CHANNEL, { replyToMessageId: "root" }),
  item("dm", "2026-11-29T08:00:00Z", { kind: "conversation", conversationId: "d1" }),
];

beforeEach(() => {
  port.items = ITEMS;
  vi.clearAllMocks();
});

describe("what waits over a field", () => {
  const waiting = (place: ScheduledPlace | null) => renderHook(() => useWaitingHere(place)).result.current;

  it("counts a channel's threads in the channel, a thread's own in the thread, and nothing that failed", () => {
    expect(waiting({ kind: "channel", channelId: "c1" })).toEqual({
      count: 2,
      next: new Date("2026-11-27T08:00:00Z"),
    });
    expect(waiting({ kind: "channel", channelId: "c1", threadRootId: "root" })).toEqual({
      count: 1,
      next: new Date("2026-11-28T08:00:00Z"),
    });
    expect(waiting({ kind: "conversation", conversationId: "d1" })).toEqual({
      count: 1,
      next: new Date("2026-11-29T08:00:00Z"),
    });
    expect(waiting({ kind: "channel", channelId: "elsewhere" })).toEqual({ count: 0, next: null });
    // An encrypted conversation is no place to schedule for.
    expect(waiting(null)).toEqual({ count: 0, next: null });
  });
});

describe("the scheduled list's actions", () => {
  it("send, edit and move a message through the port at once", async () => {
    const { result } = renderHook(() => useScheduledListActions(), { wrapper: ConfirmProvider });
    const at = new Date("2026-11-30T08:00:00Z");

    await act(() => result.current.onSendNow("failed"));
    await act(() => result.current.onEdit("channel", "nowa treść"));
    await act(() => result.current.onReschedule("thread", at));

    expect(port.sendNow).toHaveBeenCalledExactlyOnceWith("failed");
    expect(port.edit.mock.calls).toEqual([
      ["channel", { content: "nowa treść" }],
      ["thread", { sendAt: at }],
    ]);
  });

  it("deletes only once the question card is answered", async () => {
    const user = userEvent.setup();
    const { result } = renderHook(() => useScheduledListActions(), { wrapper: ConfirmProvider });

    let deleting: Promise<unknown> = Promise.resolve();
    act(() => {
      deleting = result.current.onDelete("dm");
    });
    expect(await screen.findByRole("dialog", { name: "Usunąć zaplanowaną wiadomość?" })).toBeInTheDocument();
    expect(port.remove).not.toHaveBeenCalled();

    await answerConfirm(user, "Usuń");
    await act(() => deleting);

    expect(port.remove).toHaveBeenCalledExactlyOnceWith("dm");
  });
});
