import { describe, expect, it, vi } from "vitest";

import type { SocialApiClient } from "@/lib/api/client";
import type { ConversationEncryption, Message, SocialState } from "@/lib/api/types";
import type { E2eeEvent } from "@/lib/e2ee/client";
import { FakeRealtimeClient } from "@/test/fakes";

import { createFriendsAdapter, type DmCodec } from "./friends";

const CONVERSATION_ID = "direct-1";
const PEER = { id: "user-ada", username: "ada", displayName: "Ada", presence: "online" as const };
const SELF = "user-me";
const ON: ConversationEncryption = {
  state: "on",
  requestedById: null,
  requestedAt: null,
  enabledAt: "2026-11-18T10:00:00.000Z",
  currentEpochId: "epoch-1",
};

const message = (id: string, overrides: Partial<Message> = {}): Message => ({
  id,
  channelId: CONVERSATION_ID,
  author: { id: PEER.id, displayName: PEER.displayName },
  content: "",
  attachments: [],
  embeds: [],
  createdAt: `2026-11-18T10:0${id.length % 10}:00.000Z`,
  ...overrides,
});

function state(encryption?: ConversationEncryption, lastMessage?: Message): SocialState {
  return {
    friends: [],
    requests: [],
    blocked: [],
    conversations: [
      {
        id: CONVERSATION_ID,
        peer: PEER,
        unread: 0,
        updatedAt: "2026-11-18T09:00:00.000Z",
        ...(encryption === undefined ? {} : { encryption }),
        ...(lastMessage === undefined ? {} : { lastMessage }),
      },
    ],
  };
}

function api(social: SocialState): SocialApiClient {
  return {
    getSocialState: vi.fn(async () => social),
    sendFriendRequest: vi.fn(async () => "sent" as const),
    acceptFriendRequest: vi.fn(async () => {}),
    deleteFriendRequest: vi.fn(async () => {}),
    removeFriend: vi.fn(async () => {}),
    blockUser: vi.fn(async () => {}),
    unblockUser: vi.fn(async () => {}),
    openDirectConversation: vi.fn(async () => CONVERSATION_ID),
    listDirectMessages: vi.fn(async () => ({ messages: [], nextCursor: null })),
    sendDirectMessage: vi.fn(async (_id, content) => message("plain-sent", { author: { id: SELF, displayName: "Me" }, content })),
    uploadAttachment: vi.fn(async (file: File) => ({ id: `plain-${file.name}`, url: "/a", name: file.name, contentType: "text/plain", size: 1 })),
    markDirectConversationRead: vi.fn(async () => {}),
  };
}

/** A codec that reads `ciphertext` as the plaintext and seals what it is told to. */
function codec() {
  const listeners = new Set<(event: E2eeEvent) => void>();
  const sealed = new Set<string>();
  const fake = {
    sealed,
    emit: (event: E2eeEvent) => listeners.forEach((listener) => listener(event)),
    decode: vi.fn(async (wire: Message): Promise<Message> => {
      if (wire.encrypted === undefined) return wire;
      const { encrypted, ...rest } = wire;
      return sealed.has(wire.id)
        ? { ...rest, sealed: { reason: "pin", count: 1 } }
        : { ...rest, content: encrypted.ciphertext };
    }),
    send: vi.fn(async (_id: string, input: { text: string }) =>
      message("encrypted-sent", { author: { id: SELF, displayName: "Me" }, content: input.text }),
    ),
    uploadFile: vi.fn(async (file: File) => ({
      attachmentIds: [`sealed-${file.name}`],
      payload: { id: `sealed-${file.name}`, key: "k", iv: "i", name: file.name, type: file.type, size: file.size },
    })),
    openAttachment: vi.fn(async () => ({ url: "blob:x", type: "application/octet-stream" })),
    subscribe: (listener: (event: E2eeEvent) => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  return fake satisfies DmCodec;
}

const encrypted = (id: string, text: string, overrides: Partial<Message> = {}) =>
  message(id, { encrypted: { epochId: "epoch-1", iv: "iv", ciphertext: text }, ...overrides });

describe("friends adapter with encryption", () => {
  it("keeps lines about keys in the history but out of the list's last message, order and unread count", async () => {
    const realtime = new FakeRealtimeClient();
    const friends = createFriendsAdapter(api(state(ON, message("m-0", { content: "hej" }))), realtime, codec());
    await friends.load();
    await friends.loadConversation(CONVERSATION_ID);
    const before = friends.listConversations()[0];

    realtime.push("DirectMessageCreated", message("line-device", { createdAt: "2026-11-18T11:00:00.000Z", line: { kind: "device_added", actorId: PEER.id } }));
    realtime.push("DirectMessageCreated", message("line-reset", { createdAt: "2026-11-18T11:01:00.000Z", line: { kind: "keys_reset", actorId: PEER.id } }));

    const after = friends.listConversations()[0];
    expect(after?.lastMessage?.id).toBe("m-0");
    expect(after?.updatedAt).toBe(before?.updatedAt);
    expect(after?.unread).toBe(0);
    expect(friends.listDmMessages(CONVERSATION_ID).map((item) => item.id)).toEqual(["line-device", "line-reset"]);

    realtime.push("DirectMessageCreated", message("line-request", { createdAt: "2026-11-18T11:02:00.000Z", line: { kind: "encryption_requested", actorId: PEER.id } }));
    expect(friends.listConversations()[0]).toMatchObject({ unread: 1, updatedAt: "2026-11-18T11:02:00.000Z" });
    expect(friends.listConversations()[0]?.lastMessage?.id).toBe("line-request");
  });

  it("decodes the list's last message, the history and arrivals, so nothing above holds ciphertext", async () => {
    const realtime = new FakeRealtimeClient();
    const client = api(state(ON, encrypted("m-last", "ostatnia")));
    vi.mocked(client.listDirectMessages).mockResolvedValue({ messages: [encrypted("m-1", "z historii")], nextCursor: null });
    const friends = createFriendsAdapter(client, realtime, codec());
    await friends.load();
    await friends.loadConversation(CONVERSATION_ID);

    realtime.push("DirectMessageCreated", encrypted("m-22", "nowa", { createdAt: "2026-11-18T12:00:00.000Z" }));
    await vi.waitFor(() => expect(friends.listDmMessages(CONVERSATION_ID)).toHaveLength(2));

    expect(friends.listConversations()[0]?.lastMessage?.content).toBe("nowa");
    expect(friends.listDmMessages(CONVERSATION_ID).map((item) => [item.content, item.encrypted])).toEqual([
      ["z historii", undefined],
      ["nowa", undefined],
    ]);
  });

  it("decodes a sealed message again once keys arrive", async () => {
    const realtime = new FakeRealtimeClient();
    const fake = codec();
    fake.sealed.add("m-1");
    const client = api(state(ON));
    vi.mocked(client.listDirectMessages).mockResolvedValue({ messages: [encrypted("m-1", "sekret")], nextCursor: null });
    const friends = createFriendsAdapter(client, realtime, fake);
    await friends.load();
    await friends.loadConversation(CONVERSATION_ID);
    expect(friends.listDmMessages(CONVERSATION_ID)[0]?.sealed).toEqual({ reason: "pin", count: 1 });

    fake.sealed.clear();
    fake.emit({ type: "keys" });

    await vi.waitFor(() => expect(friends.listDmMessages(CONVERSATION_ID)[0]?.content).toBe("sekret"));
  });

  it("encrypts uploads and sends through the codec while on, and plainly while off", async () => {
    const fake = codec();
    const onClient = api(state(ON));
    const on = createFriendsAdapter(onClient, new FakeRealtimeClient(), fake);
    await on.load();
    const upload = await on.uploadAttachment(CONVERSATION_ID, new File(["x"], "a.txt", { type: "text/plain" }));
    await on.sendDm(CONVERSATION_ID, { content: "tajne", attachmentIds: [upload.id], clientMessageId: "c-1" });
    expect(fake.send).toHaveBeenCalledWith(CONVERSATION_ID, expect.objectContaining({ text: "tajne", clientMessageId: "c-1" }));
    expect(onClient.sendDirectMessage).not.toHaveBeenCalled();
    expect(onClient.uploadAttachment).not.toHaveBeenCalled();

    const offClient = api(state(undefined));
    const off = createFriendsAdapter(offClient, new FakeRealtimeClient(), fake);
    await off.load();
    await off.sendDm(CONVERSATION_ID, { content: "jawne", attachmentIds: [], clientMessageId: "c-2" });
    expect(offClient.sendDirectMessage).toHaveBeenCalledWith(CONVERSATION_ID, "jawne", [], "c-2");
  });

  it("refuses a plain upload in an encrypted message before anything is sent", async () => {
    const fake = codec();
    const friends = createFriendsAdapter(api(state(ON)), new FakeRealtimeClient(), fake);
    await friends.load();

    await expect(
      friends.sendDm(CONVERSATION_ID, { content: "x", attachmentIds: ["plain-upload"], clientMessageId: "c-1" }),
    ).rejects.toMatchObject({ code: "attachment-kind-mismatch" });
    expect(fake.send).not.toHaveBeenCalled();
  });

  it("follows the switch from the hub", async () => {
    const realtime = new FakeRealtimeClient();
    const friends = createFriendsAdapter(api(state(undefined)), realtime, codec());
    await friends.load();

    realtime.push("DirectEncryptionChanged", { conversationId: CONVERSATION_ID, encryption: ON });

    expect(friends.listConversations()[0]?.encryption?.state).toBe("on");
  });
});
