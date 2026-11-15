import { describe, expect, it } from "vitest";

import { buildTranscript } from "@/lib/format";
import type { Message, Reaction } from "@/ports";
import { at, message } from "@/test/messages";

import {
  createTranscriptPresenter,
  presentTranscript,
  type PresentationContext,
} from "./present";

const NOW = new Date("2026-01-01T13:00:00Z");
const BOB = { id: "u-bob", displayName: "Bob" };
const CAROL = { id: "u-carol", displayName: "Carol" };

function context(overrides: Partial<PresentationContext> = {}): PresentationContext {
  return {
    selfId: "u-ada",
    server: { name: "Orbit", iconUrl: "/orbit.png" },
    faces: new Map([["u-ada", "/a.png"]]),
    customEmoji: new Map(),
    mentions: new Map(),
    pinnedIds: new Set(),
    reactionsOf: () => [],
    extFallback: "FILE",
    ...overrides,
  };
}

function transcriptOf(messages: Message[]) {
  return buildTranscript(messages, NOW, "en");
}

type Sections = ReturnType<typeof presentTranscript>;

const groupsOf = (sections: Sections) => sections.flatMap((section) => section.groups);

const bodiesOf = (sections: Sections) =>
  groupsOf(sections).flatMap((group) => group.bodies);

describe("createTranscriptPresenter", () => {
  it("reuses the day section and the whole result while nothing changed", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);

    const initial = present(transcriptOf([first]), view);
    const again = present(transcriptOf([first]), view);

    expect(again).toBe(initial);
    expect(again[0]).toBe(initial[0]);
  });

  it("reuses an unchanged day section when a later day arrives", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);
    const nextDay = message("m2", 1, {
      createdAt: new Date(Date.UTC(2026, 0, 2, 12, 1)).toISOString(),
    });

    const initial = present(transcriptOf([first]), view);
    const appended = present(transcriptOf([first, nextDay]), view);

    expect(appended).not.toBe(initial);
    expect(appended[0]).toBe(initial[0]);
    expect(appended[1]?.groups[0]?.bodies[0]?.id).toBe("m2");
  });

  it("reuses the rows already shown when a message arrives", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);
    const second = message("m2", 2, { author: BOB });

    const initial = present(transcriptOf([first]), view);
    const [initialGroup] = groupsOf(initial);
    const initialBody = initialGroup?.bodies[0];
    const initialParts = initialBody?.parts;

    const appended = present(transcriptOf([first, second]), view);
    const [firstGroup, secondGroup] = groupsOf(appended);

    expect(firstGroup).toBe(initialGroup);
    expect(firstGroup?.bodies[0]).toBe(initialBody);
    expect(firstGroup?.bodies[0]?.parts).toBe(initialParts);
    expect(secondGroup?.bodies.map((body) => body.id)).toEqual(["m2"]);
  });

  it("reuses the existing bodies when the arrival joins their block", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);
    const second = message("m2", 2);

    const initialGroup = groupsOf(present(transcriptOf([first, second]), view))[0];
    const [initialFirst, initialSecond] = initialGroup?.bodies ?? [];
    const initialParts = initialFirst?.parts;

    const grown = groupsOf(
      present(transcriptOf([first, second, message("m3", 3)]), view),
    )[0];

    expect(grown).not.toBe(initialGroup);
    expect(grown?.bodies[0]).toBe(initialFirst);
    expect(grown?.bodies[0]?.parts).toBe(initialParts);
    expect(grown?.bodies[1]).toBe(initialSecond);
    expect(grown?.bodies[2]?.id).toBe("m3");
  });

  it("reuses every later block when an older message is prepended", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 10);
    const second = message("m2", 11, { author: BOB });

    const initial = present(transcriptOf([first, second]), view);
    const [initialFirst, initialSecond] = groupsOf(initial);
    const initialParts = initialFirst?.bodies[0]?.parts;

    const prepended = present(
      transcriptOf([message("m0", 1, { author: CAROL }), first, second]),
      view,
    );
    const [older, firstGroup, secondGroup] = groupsOf(prepended);

    expect(older?.authorId).toBe(CAROL.id);
    expect(firstGroup).toBe(initialFirst);
    expect(firstGroup?.bodies[0]?.parts).toBe(initialParts);
    expect(secondGroup).toBe(initialSecond);
  });

  it("replaces only the body of a message that was edited", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);
    const second = message("m2", 2, { author: BOB });

    const initial = present(transcriptOf([first, second]), view);
    const [initialFirst, initialSecond] = groupsOf(initial);
    const initialParts = initialFirst?.bodies[0]?.parts;

    const edited = { ...first, content: "a corrected sentence", editedAt: at(3) };
    const updated = present(transcriptOf([edited, second]), view);
    const [editedGroup, untouchedGroup] = groupsOf(updated);

    expect(editedGroup).not.toBe(initialFirst);
    expect(editedGroup?.bodies[0]?.parts).not.toBe(initialParts);
    expect(JSON.stringify(editedGroup?.bodies[0]?.parts)).toContain("corrected sentence");
    expect(untouchedGroup).toBe(initialSecond);
  });

  it("keeps a settled message's old id out of every row", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const placeholder = message("local-1", 1);

    const initial = present(transcriptOf([placeholder]), view);
    const initialBody = bodiesOf(initial)[0];
    const settled = { ...placeholder, id: "server-1" };

    const next = present(transcriptOf([settled]), view);
    const [body] = bodiesOf(next);

    expect(body?.id).toBe("server-1");
    expect(body).not.toBe(initialBody);
    expect(bodiesOf(next).some((item) => item.id === "local-1")).toBe(false);
  });

  it("keeps the remaining blocks when one is deleted", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);
    const second = message("m2", 2, { author: BOB });
    const third = message("m3", 3, { author: CAROL });

    const initial = present(transcriptOf([first, second, third]), view);
    const [initialFirst, , initialThird] = groupsOf(initial);
    const firstParts = initialFirst?.bodies[0]?.parts;

    const afterDelete = present(transcriptOf([first, third]), view);
    const [firstGroup, thirdGroup] = groupsOf(afterDelete);

    expect(firstGroup).toBe(initialFirst);
    expect(firstGroup?.bodies[0]?.parts).toBe(firstParts);
    expect(thirdGroup).toBe(initialThird);
  });

  it("rebuilds a block's metadata, not its body, when a face or the reader changes", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);
    const second = message("m2", 2, { author: BOB });

    const initial = present(transcriptOf([first, second]), view);
    const [initialFirst, initialSecond] = groupsOf(initial);
    const initialBody = initialFirst?.bodies[0];
    const initialParts = initialBody?.parts;

    const changed = present(transcriptOf([first, second]), {
      ...view,
      selfId: undefined,
      faces: new Map([["u-ada", "/new.png"]]),
    });
    const [firstGroup, secondGroup] = groupsOf(changed);

    expect(firstGroup).not.toBe(initialFirst);
    expect(firstGroup?.authorAvatarUrl).toBe("/new.png");
    expect(firstGroup?.own).toBeUndefined();
    expect(firstGroup?.bodies[0]).toBe(initialBody);
    expect(firstGroup?.bodies[0]?.parts).toBe(initialParts);
    expect(secondGroup).toBe(initialSecond);
  });

  it("refreshes a server notice's identity without reparsing its body", () => {
    const present = createTranscriptPresenter();
    const notice = message("n1", 1, { author: { id: "system", displayName: "system" } });
    const view = context();

    const initial = groupsOf(present(transcriptOf([notice]), view))[0];
    const initialBody = initial?.bodies[0];

    const renamed = groupsOf(
      present(transcriptOf([notice]), {
        ...view,
        server: { name: "Elsewhere", iconUrl: "/elsewhere.png" },
      }),
    )[0];

    expect(renamed).not.toBe(initial);
    expect(renamed).toMatchObject({
      authorName: "Elsewhere",
      authorAvatarUrl: "/elsewhere.png",
      system: true,
    });
    expect(renamed?.bodies[0]).toBe(initialBody);
  });

  it("reparses when custom emoji, mentions or the file fallback change", () => {
    const present = createTranscriptPresenter();
    const emojiMessage = message("m1", 1, { content: ":party: hello @bob" });
    const mention = {
      userId: "u-bob",
      username: "bob",
      displayName: "Bob",
      self: false,
    };
    const fileMessage = message("f1", 2, {
      content: "",
      attachments: [
        {
          id: "a1",
          url: "https://cdn/a1",
          name: "README",
          contentType: "application/octet-stream",
          size: 12,
        },
      ],
    });
    const view = context({
      customEmoji: new Map([["party", "/party-a.png"]]),
      mentions: new Map([["bob", mention]]),
    });
    const transcript = transcriptOf([emojiMessage, fileMessage]);
    const initial = present(transcript, view);
    const [emojiBody, fileBody] = bodiesOf(initial);

    const emojiChanged = present(transcript, {
      ...view,
      customEmoji: new Map([["party", "/party-b.png"]]),
    });
    const firstAfterEmoji = bodiesOf(emojiChanged)[0];
    expect(firstAfterEmoji?.parts).not.toBe(emojiBody?.parts);
    expect(JSON.stringify(firstAfterEmoji?.parts)).toContain("/party-b.png");

    const mentionsChanged = present(transcript, { ...view, mentions: new Map() });
    expect(bodiesOf(mentionsChanged)[0]?.parts).not.toBe(emojiBody?.parts);

    const fallbackChanged = present(transcript, {
      ...view,
      extFallback: "PLIK",
    });
    expect(bodiesOf(fallbackChanged)[1]?.parts).not.toBe(fileBody?.parts);
    expect(bodiesOf(fallbackChanged)[1]?.parts).toMatchObject([{ kind: "file", ext: "PLIK" }]);
  });

  it("derives reactions and pins on every call and rebuilds only their body", () => {
    const present = createTranscriptPresenter();
    const first = message("m1", 1);
    const second = message("m2", 2, { author: BOB });
    const byMessage = new Map<string, Reaction[]>([
      ["m1", [{ emoji: "👍", count: 1, mine: false }]],
    ]);
    // The real port copies its aggregate on every read.
    const view = context({ reactionsOf: (id) => [...(byMessage.get(id) ?? [])] });

    const transcript = transcriptOf([first, second]);
    const initial = present(transcript, view);
    const [initialFirst, initialSecond] = groupsOf(initial);
    const firstParts = initialFirst?.bodies[0]?.parts;
    const secondParts = initialSecond?.bodies[0]?.parts;

    byMessage.set("m1", [{ emoji: "👍", count: 2, mine: true }]);
    const reacted = present(transcript, view);
    const [reactedFirst, reactedSecond] = groupsOf(reacted);

    expect(reactedFirst).not.toBe(initialFirst);
    expect(reactedFirst?.bodies[0]?.reactions).toEqual([
      { emoji: "👍", count: 2, mine: true },
    ]);
    expect(reactedFirst?.bodies[0]?.parts).toBe(firstParts);
    expect(reactedSecond).toBe(initialSecond);

    const pinned = present(transcript, {
      ...view,
      pinnedIds: new Set(["m2"]),
    });
    const [pinnedFirst, pinnedSecond] = groupsOf(pinned);

    expect(pinnedFirst).toBe(reactedFirst);
    expect(pinnedSecond).not.toBe(reactedSecond);
    expect(pinnedSecond?.bodies[0]?.pinned).toBe(true);
    expect(pinnedSecond?.bodies[0]?.parts).toBe(secondParts);

    const sameAgain = present(transcript, {
      ...view,
      pinnedIds: new Set(["m2"]),
    });
    expect(groupsOf(sameAgain)).toEqual([pinnedFirst, pinnedSecond]);
    expect(groupsOf(sameAgain)[0]).toBe(pinnedFirst);
    expect(groupsOf(sameAgain)[1]).toBe(pinnedSecond);
  });

  it("keeps each presenter's cache to itself", () => {
    const one = createTranscriptPresenter();
    const two = createTranscriptPresenter();
    const view = context();
    const transcript = transcriptOf([message("m1", 1)]);

    const first = bodiesOf(one(transcript, view))[0];
    const second = bodiesOf(two(transcript, view))[0];

    expect(second).not.toBe(first);
    expect(second?.parts).not.toBe(first?.parts);
  });

  it("releases the rows of a channel that was switched away from", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);

    const initial = bodiesOf(present(transcriptOf([first]), view))[0];
    const initialParts = initial?.parts;

    present(transcriptOf([message("other", 1, { channelId: "ch-other" })]), view);
    const returned = bodiesOf(present(transcriptOf([first]), view))[0];

    expect(returned).not.toBe(initial);
    expect(returned?.parts).not.toBe(initialParts);
    expect(JSON.stringify(returned?.parts)).toBe(JSON.stringify(initialParts));
  });

  it("releases its rows when the transcript becomes empty", () => {
    const present = createTranscriptPresenter();
    const view = context();
    const first = message("m1", 1);

    const initial = bodiesOf(present(transcriptOf([first]), view))[0];
    present([], view);
    const returned = bodiesOf(present(transcriptOf([first]), view))[0];

    expect(returned).not.toBe(initial);
    expect(returned?.parts).not.toBe(initial?.parts);
  });

  it("produces the same view model as the stateless presentation", () => {
    const transcript = transcriptOf([
      message("m1", 1, { content: "hello @bob" }),
      message("m2", 2, { author: BOB, pinnedAt: at(4) }),
      message("n1", 3, { author: { id: "system", displayName: "system" } }),
    ]);
    const view = context({
      mentions: new Map([
        ["bob", { userId: "u-bob", username: "bob", displayName: "Bob", self: false }],
      ]),
      pinnedIds: new Set(["m2"]),
      reactionsOf: (id) =>
        id === "m1" ? [{ emoji: "🎉", count: 1, mine: true }] : [],
    });

    const presented = createTranscriptPresenter()(transcript, view);

    expect(presented).toEqual(presentTranscript(transcript, view));
  });
});
