import { describe, expect, it, vi } from "vitest";
import type { Message } from "@/lib/api/types";
import {
  buildThreadTranscript,
  buildTranscript,
  formatBytes,
  formatDay,
  initials,
  messageReference,
} from "./format";
import { serverEvent } from "@/test/events";

function message(
  id: string,
  authorId: string,
  createdAt: string,
  content = id,
): Message {
  return {
    id,
    channelId: "ch-general",
    author: { id: authorId, displayName: authorId },
    content,
    attachments: [],
    embeds: [],
    createdAt,
  };
}

const NOW = new Date("2026-08-10T21:00:00Z");

describe("buildTranscript", () => {
  it("returns nothing for an empty channel", () => {
    expect(buildTranscript([], NOW)).toEqual([]);
  });

  it("merges consecutive messages from one author inside the 5 minute window", () => {
    const sections = buildTranscript(
      [
        message("a", "ada", "2026-08-10T19:00:00Z"),
        message("b", "ada", "2026-08-10T19:03:00Z"),
        message("c", "ada", "2026-08-10T19:04:30Z"),
      ],
      NOW,
    );

    expect(sections).toHaveLength(1);
    expect(sections[0]?.groups).toHaveLength(1);
    expect(sections[0]?.groups[0]?.messages.map((m) => m.id)).toEqual([
      "a",
      "b",
      "c",
    ]);
  });

  it("starts a new group once the window lapses", () => {
    const sections = buildTranscript(
      [
        message("a", "ada", "2026-08-10T19:00:00Z"),
        message("b", "ada", "2026-08-10T19:06:00Z"),
      ],
      NOW,
    );

    expect(sections[0]?.groups).toHaveLength(2);
  });

  it("starts a new group when the author changes", () => {
    const sections = buildTranscript(
      [
        message("a", "ada", "2026-08-10T19:00:00Z"),
        message("b", "nils", "2026-08-10T19:00:30Z"),
        message("c", "ada", "2026-08-10T19:01:00Z"),
      ],
      NOW,
    );

    expect(sections[0]?.groups.map((group) => group.authorId)).toEqual([
      "ada",
      "nils",
      "ada",
    ]);
  });

  it("splits sections on a day boundary", () => {
    const sections = buildTranscript(
      [
        message("a", "ada", "2026-08-09T19:00:00Z"),
        message("b", "ada", "2026-08-10T19:00:00Z"),
      ],
      NOW,
    );

    expect(sections).toHaveLength(2);
    expect(sections.map((section) => section.day)).toEqual(["Wczoraj", "Dziś"]);
  });

  it("keeps unparseable timestamps in an empty day and separate groups", () => {
    const sections = buildTranscript(
      [
        message("bad-1", "ada", "not-a-date"),
        message("bad-2", "ada", "still-not-a-date"),
      ],
      NOW,
      "en",
    );

    expect(sections).toHaveLength(1);
    expect(sections[0]?.day).toBe("");
    expect(
      sections[0]?.groups.map((group) => group.messages.map((item) => item.id)),
    ).toEqual([["bad-1"], ["bad-2"]]);
  });

  it("formats each contiguous day heading once", () => {
    const firstDay = new Date(2026, 7, 1, 12, 0, 0);
    const secondDay = new Date(2026, 7, 2, 12, 0, 0);
    const at = (day: Date, minute: number) =>
      new Date(day.getTime() + minute * 60_000).toISOString();
    const firstIso = at(firstDay, 0);
    const secondIso = at(secondDay, 0);
    const messages = [
      message("a1", "ada", firstIso),
      message("a2", "ada", at(firstDay, 2)),
      message("a3", "ada", at(firstDay, 10)),
      message("a4", "nils", at(firstDay, 11)),
      message("b1", "ada", secondIso),
      message("b2", "ada", at(secondDay, 1)),
    ];
    const expectedDays = [
      formatDay(firstIso, NOW, "en"),
      formatDay(secondIso, NOW, "en"),
    ];

    const formatter = vi.spyOn(Intl, "DateTimeFormat");
    let sections: ReturnType<typeof buildTranscript> = [];
    let formats = 0;
    try {
      sections = buildTranscript(messages, NOW, "en");
      formats = formatter.mock.calls.length;
    } finally {
      formatter.mockRestore();
    }

    expect(formats).toBe(2);
    expect(sections.map((section) => section.day)).toEqual(expectedDays);
    expect(
      sections.map((section) =>
        section.groups.map((group) => group.messages.map((item) => item.id)),
      ),
    ).toEqual([
      [["a1", "a2"], ["a3"], ["a4"]],
      [["b1", "b2"]],
    ]);
  });
});

describe("messageReference", () => {
  it("quotes the words, on one line", () => {
    const answered = message("a", "ada", "2026-08-10T19:00:00Z", "no to\n  jedziemy");
    expect(messageReference(answered)).toEqual({
      id: "a",
      authorId: "ada",
      authorName: "ada",
      preview: "no to jedziemy",
    });
  });

  it("says what a wordless attachment is instead of quoting nothing", () => {
    const answered: Message = {
      ...message("a", "ada", "2026-08-10T19:00:00Z", ""),
      attachments: [
        {
          id: "att-1",
          url: "/u/a.png",
          name: "a.png",
          contentType: "image/png",
          size: 1024,
        },
      ],
    };
    expect(messageReference(answered)).toMatchObject({
      preview: "Kliknij, aby zobaczyć załącznik",
      hasAttachment: true,
    });
  });
});

describe("messageReference of a message with no words", () => {
  it("quotes a poll by its question and an announcement by its event's name", () => {
    const empty = message("a", "ada", "2026-08-10T19:00:00Z", "");
    const poll = { question: "Kiedy gramy?", answers: [], multiple: false, endsAt: "2026-08-11T19:00:00Z", voters: 0 };
    expect(messageReference({ ...empty, poll }).preview).toBe("Kiedy gramy?");
    expect(messageReference({ ...empty, event: serverEvent() }).preview).toBe("Wieczór planszówek");
  });
});

describe("formatDay", () => {
  it("names today and yesterday rather than printing a date", () => {
    expect(formatDay("2026-08-10T10:00:00Z", NOW)).toBe("Dziś");
    expect(formatDay("2026-08-09T10:00:00Z", NOW)).toBe("Wczoraj");
  });

  it("survives an unparseable timestamp", () => {
    expect(formatDay("not-a-date", NOW)).toBe("");
  });
});

describe("buildTranscript and replies", () => {
  it("starts a new block for a reply, even mid-conversation", () => {
    const answer: Message = {
      ...message("b", "ada", "2026-08-10T19:01:00Z"),
      replyTo: {
        id: "a",
        authorId: "nils",
        authorName: "Nils",
        preview: "pytanie",
      },
    };
    const sections = buildTranscript(
      [message("a", "ada", "2026-08-10T19:00:00Z"), answer],
      NOW,
    );

    expect(sections[0]?.groups).toHaveLength(2);
    expect(sections[0]?.groups[1]?.messages[0]?.replyTo?.authorName).toBe("Nils");
  });

  it("does not make a later channel message look like part of the reply", () => {
    const answer: Message = {
      ...message("answer", "ada", "2026-08-10T19:01:00Z"),
      replyTo: {
        id: "root",
        authorId: "nils",
        authorName: "Nils",
        preview: "pytanie",
      },
    };
    const sections = buildTranscript(
      [answer, message("channel", "ada", "2026-08-10T19:02:00Z")],
      NOW,
    );

    expect(sections[0]?.groups).toHaveLength(2);
    expect(sections[0]?.groups[1]?.messages[0]?.id).toBe("channel");
    expect(sections[0]?.groups[1]?.messages[0]?.replyTo).toBeUndefined();
  });

  it("builds a thread from the root and its exact replies as a normal discussion", () => {
    const root = message("root", "ada", "2026-08-10T19:00:00Z");
    const reply = (id: string, at: string): Message => ({
      ...message(id, "ada", at),
      replyTo: {
        id: root.id,
        authorId: root.author.id,
        authorName: root.author.displayName,
        preview: root.content,
      },
    });
    const sections = buildThreadTranscript(
      [
        root,
        reply("reply-1", "2026-08-10T19:01:00Z"),
        message("ordinary-channel-message", "ada", "2026-08-10T19:02:00Z"),
        reply("reply-2", "2026-08-10T19:03:00Z"),
      ],
      root.id,
      NOW,
    );

    expect(sections[0]?.groups).toHaveLength(2);
    expect(sections[0]?.groups[0]?.messages.map(({ id }) => id)).toEqual(["root"]);
    expect(sections[0]?.groups[1]?.messages.map(({ id }) => id)).toEqual([
      "reply-1",
      "reply-2",
    ]);
    expect(sections[0]?.groups[1]?.messages[0]?.replyTo).toBeUndefined();
  });
});

describe("initials", () => {
  it("takes one letter per word, or two from a single word", () => {
    expect(initials("Ada")).toBe("AD");
    expect(initials("Ada Lovelace")).toBe("AL");
    expect(initials("  ")).toBe("?");
  });
});

describe("formatBytes", () => {
  it("scales the unit to the size", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2 KB");
    expect(formatBytes(3_500_000)).toBe("3.3 MB");
  });
});
