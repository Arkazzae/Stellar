import { describe, expect, it } from "vitest";

import { shouldPlayMessageNotificationSound, shouldShowMentionNotification } from "./notifications";

const sound = {
  authorId: "user-other",
  channelId: "channel-general",
  selfId: "user-self",
  selfUsername: "ada",
  openChannelId: "channel-general",
  soundEnabled: true,
  mentionsOnly: true,
  mutedChannelIds: [] as string[],
};

const popup = {
  authorId: "user-other",
  channelId: "channel-general",
  selfId: "user-self",
  selfUsername: "ada",
  enabled: true,
  mutedChannelIds: [] as string[],
  away: true,
};

describe("a call on a group", () => {
  it("rings and pops up only when the server stamped the message", () => {
    for (const content of ["@everyone", "@wszyscy", "@here", "@obecni"]) {
      expect(shouldPlayMessageNotificationSound({ ...sound, content }), content).toBe(false);
      expect(shouldShowMentionNotification({ ...popup, content }), content).toBe(false);
    }
    expect(shouldPlayMessageNotificationSound({ ...sound, content: "@everyone", mentionsEveryone: true })).toBe(true);
    expect(shouldShowMentionNotification({ ...popup, content: "@obecni", mentionsHere: true })).toBe(true);
  });

  it("counts as a mention for whoever only wants to hear mentions", () => {
    expect(
      shouldPlayMessageNotificationSound({ ...sound, content: "spotkanie @wszyscy", mentionsEveryone: true }),
    ).toBe(true);
  });

  it("leaves out whoever appears offline when it calls on those here", () => {
    const here = { content: "@here", mentionsHere: true, selfPresence: "offline" as const };

    expect(shouldPlayMessageNotificationSound({ ...sound, ...here })).toBe(false);
    expect(shouldShowMentionNotification({ ...popup, ...here })).toBe(false);
    expect(shouldPlayMessageNotificationSound({ ...sound, ...here, selfPresence: "idle" })).toBe(true);
    // A call on everyone reaches them all the same.
    expect(
      shouldShowMentionNotification({ ...popup, content: "@everyone", mentionsEveryone: true, selfPresence: "offline" }),
    ).toBe(true);
  });

  it("keeps quiet in a muted channel, for the author, and while the window has focus", () => {
    const stamped = { content: "@everyone", mentionsEveryone: true };

    expect(shouldPlayMessageNotificationSound({ ...sound, ...stamped, mutedChannelIds: ["channel-general"] })).toBe(false);
    expect(shouldPlayMessageNotificationSound({ ...sound, ...stamped, authorId: "user-self" })).toBe(false);
    expect(shouldShowMentionNotification({ ...popup, ...stamped, away: false })).toBe(false);
    expect(shouldShowMentionNotification({ ...popup, ...stamped, enabled: false })).toBe(false);
  });

  it("does not make a reader whose username is a group's word mentioned by the word", () => {
    expect(shouldPlayMessageNotificationSound({ ...sound, selfUsername: "everyone", content: "@everyone" })).toBe(false);
  });

  it("rings for a call at the end of a sentence by its stamp, and still for a member named by the dotted word", () => {
    const content = "Spotkanie o 20 @wszyscy.";

    expect(shouldPlayMessageNotificationSound({ ...sound, content })).toBe(false);
    expect(shouldShowMentionNotification({ ...popup, content, mentionsEveryone: true })).toBe(true);
    // A username may end with a dot; whoever is called `wszyscy.` is named.
    expect(shouldPlayMessageNotificationSound({ ...sound, selfUsername: "wszyscy.", content })).toBe(true);
  });
});
