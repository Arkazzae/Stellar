import { describe, expect, it, vi } from "vitest";

import type { SocialApiClient } from "@/lib/api/client";
import type { SocialState } from "@/lib/api/types";
import { FakeRealtimeClient } from "@/test/fakes";

import { createFriendsAdapter } from "./friends";

const CONVERSATION_ID = "direct-1";
const PEER = {
  id: "user-ada",
  username: "ada",
  displayName: "Ada",
  presence: "online" as const,
};
const STATE: SocialState = {
  friends: [{ user: PEER, since: "2026-01-01T00:00:00.000Z" }],
  requests: [],
  blocked: [],
  conversations: [
    {
      id: CONVERSATION_ID,
      peer: PEER,
      unread: 0,
      updatedAt: "2026-01-01T00:00:00.000Z",
    },
  ],
};

function api(): SocialApiClient {
  return {
    getSocialState: vi.fn(async () => STATE),
    sendFriendRequest: vi.fn(async () => "sent" as const),
    acceptFriendRequest: vi.fn(async () => {}),
    deleteFriendRequest: vi.fn(async () => {}),
    removeFriend: vi.fn(async () => {}),
    blockUser: vi.fn(async () => {}),
    unblockUser: vi.fn(async () => {}),
    openDirectConversation: vi.fn(async () => CONVERSATION_ID),
    listDirectMessages: vi.fn(async () => ({ messages: [], nextCursor: null })),
    sendDirectMessage: vi.fn(async () => {
      throw new Error("not used");
    }),
    uploadAttachment: vi.fn(async () => {
      throw new Error("not used");
    }),
    markDirectConversationRead: vi.fn(async () => {}),
  };
}

describe("friends adapter realtime", () => {
  it("loads a conversation opened from the rail once, and not again", async () => {
    const client = api();
    const message = {
      id: "dm-1",
      channelId: CONVERSATION_ID,
      author: { id: PEER.id, displayName: PEER.displayName },
      content: "z historii",
      attachments: [],
      embeds: [],
      createdAt: "2026-01-01T00:00:00.000Z",
    };
    vi.mocked(client.listDirectMessages).mockResolvedValue({
      messages: [message],
      nextCursor: null,
    });
    const friends = createFriendsAdapter(client, new FakeRealtimeClient());

    const opening = friends.loadConversation(CONVERSATION_ID);
    const sameOpening = friends.loadConversation(CONVERSATION_ID);
    await Promise.all([opening, sameOpening]);
    await friends.loadConversation(CONVERSATION_ID);

    expect(client.listDirectMessages).toHaveBeenCalledTimes(1);
    expect(client.listDirectMessages).toHaveBeenCalledWith(CONVERSATION_ID, { limit: 50 });
    expect(friends.listDmMessages(CONVERSATION_ID).map((item) => item.content)).toEqual([
      "z historii",
    ]);
    friends.dispose?.();
  });

  it("loads older DM pages without replacing the newest page", async () => {
    const realtime = new FakeRealtimeClient();
    const client = api();
    const newest = {
      id: "dm-new",
      channelId: CONVERSATION_ID,
      author: { id: PEER.id, displayName: PEER.displayName },
      content: "newest",
      attachments: [],
      embeds: [],
      createdAt: "2026-01-02T00:00:00.000Z",
    };
    const oldest = {
      ...newest,
      id: "dm-old",
      content: "oldest",
      createdAt: "2026-01-01T00:00:00.000Z",
    };
    vi.mocked(client.listDirectMessages)
      .mockResolvedValueOnce({ messages: [newest], nextCursor: "older-page" })
      .mockResolvedValueOnce({ messages: [oldest], nextCursor: null });
    const friends = createFriendsAdapter(client, realtime);

    await friends.openConversation(PEER.id);
    expect(friends.hasOlderDmMessages(CONVERSATION_ID)).toBe(true);
    await friends.loadOlderDmMessages(CONVERSATION_ID);

    expect(client.listDirectMessages).toHaveBeenLastCalledWith(
      CONVERSATION_ID,
      { cursor: "older-page", limit: 50 },
    );
    expect(friends.listDmMessages(CONVERSATION_ID).map((message) => message.id))
      .toEqual(["dm-old", "dm-new"]);
    expect(friends.hasOlderDmMessages(CONVERSATION_ID)).toBe(false);
    friends.dispose?.();
  });

  it("continues DM pagination from the fresh snapshot after reconnecting", async () => {
    const realtime = new FakeRealtimeClient();
    const client = api();
    const message = {
      id: "dm-new",
      channelId: CONVERSATION_ID,
      author: { id: PEER.id, displayName: PEER.displayName },
      content: "newest",
      attachments: [],
      embeds: [],
      createdAt: "2026-01-02T00:00:00.000Z",
    };
    vi.mocked(client.listDirectMessages)
      .mockResolvedValueOnce({ messages: [message], nextCursor: "stale-snapshot" })
      .mockResolvedValueOnce({ messages: [message], nextCursor: "fresh-snapshot" })
      .mockResolvedValueOnce({ messages: [], nextCursor: null });
    const friends = createFriendsAdapter(client, realtime);

    await friends.openConversation(PEER.id);
    await realtime.start();
    await vi.waitFor(() => expect(client.listDirectMessages).toHaveBeenCalledTimes(2));
    await friends.loadOlderDmMessages(CONVERSATION_ID);

    expect(client.listDirectMessages).toHaveBeenLastCalledWith(
      CONVERSATION_ID,
      { cursor: "fresh-snapshot", limit: 50 },
    );
    friends.dispose?.();
  });

  it("reports a read once per newest message, not on every refresh the read causes", async () => {
    const client = api();
    const realtime = new FakeRealtimeClient();
    const friends = createFriendsAdapter(client, realtime);
    await friends.load();

    friends.markRead(CONVERSATION_ID);
    // The server answers every read with SocialChanged and a fresh snapshot.
    for (let refresh = 0; refresh < 3; refresh++) {
      realtime.push("SocialChanged", {});
      await friends.load();
      friends.markRead(CONVERSATION_ID);
    }
    expect(client.markDirectConversationRead).toHaveBeenCalledTimes(1);

    realtime.push("DirectMessageCreated", {
      id: "dm-2",
      channelId: CONVERSATION_ID,
      author: { id: PEER.id, displayName: PEER.displayName },
      content: "nowa",
      attachments: [],
      embeds: [],
      createdAt: "2026-01-01T02:00:00.000Z",
    });
    friends.markRead(CONVERSATION_ID);
    expect(client.markDirectConversationRead).toHaveBeenCalledTimes(2);
    friends.dispose?.();
  });

  it("appends a pushed DM and updates its conversation summary", async () => {
    const realtime = new FakeRealtimeClient();
    const friends = createFriendsAdapter(api(), realtime);
    await friends.load();
    await friends.openConversation(PEER.id);

    realtime.push("DirectMessageCreated", {
      id: "dm-1",
      channelId: CONVERSATION_ID,
      author: { id: PEER.id, displayName: PEER.displayName },
      content: "wiadomość na żywo",
      attachments: [],
      embeds: [],
      createdAt: "2026-01-01T01:00:00.000Z",
    });

    expect(friends.listDmMessages(CONVERSATION_ID)).toHaveLength(1);
    expect(friends.listConversations()[0]).toMatchObject({
      unread: 1,
      lastMessage: { id: "dm-1", content: "wiadomość na żywo" },
    });
    friends.dispose?.();
  });

  it("projects private RTC occupancy onto the matching DM only", async () => {
    const realtime = new FakeRealtimeClient();
    const friends = createFriendsAdapter(api(), realtime);
    await friends.load();
    const participants = [
      { userId: PEER.id, displayName: PEER.displayName, muted: false },
    ];

    realtime.push("VoiceParticipantChanged", {
      channelId: CONVERSATION_ID,
      participants,
    });
    expect(friends.listConversations()[0]?.callParticipants).toEqual(
      participants,
    );
    const firstCallId = friends.listConversations()[0]?.callId;

    realtime.push("VoiceParticipantChanged", {
      channelId: CONVERSATION_ID,
      participants: [{ ...participants[0]!, muted: true }],
    });
    expect(friends.listConversations()[0]?.callId).toBe(firstCallId);

    realtime.push("VoiceParticipantChanged", {
      channelId: CONVERSATION_ID,
      participants: [],
    });
    expect(friends.listConversations()[0]?.callParticipants).toBeUndefined();

    realtime.push("VoiceParticipantChanged", {
      channelId: CONVERSATION_ID,
      participants,
    });
    expect(friends.listConversations()[0]?.callId).not.toBe(firstCallId);
    friends.dispose?.();
  });
});

describe("live changes during social reads", () => {
  it("keeps pushed messages before and during the first history response without duplicates", async () => {
    const realtime = new FakeRealtimeClient();
    const client = api();
    let finish!: (page: { messages: typeof incoming[]; nextCursor: string | null }) => void;
    const incoming = {
      id: "live-1", channelId: CONVERSATION_ID,
      author: { id: PEER.id, displayName: PEER.displayName },
      content: "arrived live", attachments: [], embeds: [], createdAt: "2026-01-02T00:00:00Z",
    };
    vi.mocked(client.listDirectMessages).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const friends = createFriendsAdapter(client, realtime);
    realtime.push("DirectMessageCreated", incoming);
    const loading = friends.loadConversation(CONVERSATION_ID);
    const second = { ...incoming, id: "live-2", createdAt: "2026-01-03T00:00:00Z" };
    realtime.push("DirectMessageCreated", second);
    finish({ messages: [incoming], nextCursor: null });
    await loading;
    expect(friends.listDmMessages(CONVERSATION_ID).map((message) => message.id)).toEqual(["live-1", "live-2"]);
    friends.dispose?.();
  });

  it("updates friend and DM presence from the hub and stops on disposal", async () => {
    const realtime = new FakeRealtimeClient();
    const friends = createFriendsAdapter(api(), realtime);
    await friends.load();
    realtime.push("PresenceChanged", { userId: PEER.id, presence: "offline" });
    expect(friends.listFriends()[0]?.user.presence).toBe("offline");
    expect(friends.listConversations()[0]?.peer.presence).toBe("offline");
    friends.dispose?.();
    realtime.push("PresenceChanged", { userId: PEER.id, presence: "online" });
    expect(friends.listFriends()[0]?.user.presence).toBe("offline");
  });

  it("keeps a live presence change that arrives during the initial social snapshot", async () => {
    const realtime = new FakeRealtimeClient();
    const client = api();
    let finish!: (state: SocialState) => void;
    vi.mocked(client.getSocialState).mockImplementationOnce(
      () => new Promise((resolve) => { finish = resolve; }),
    );
    const friends = createFriendsAdapter(client, realtime);
    const loading = friends.load();
    realtime.push("PresenceChanged", { userId: PEER.id, presence: "offline" });
    finish(STATE);
    await loading;
    expect(friends.listFriends()[0]?.user.presence).toBe("offline");
    expect(friends.listConversations()[0]?.peer.presence).toBe("offline");
    friends.dispose?.();
  });
});
