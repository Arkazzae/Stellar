import { describe, expect, it } from "vitest";

import type { ConversationEncryption, Message, SealedReason, TimelineLineKind } from "@/ports";

import { createSealedCollapser, encryptionLineFacts } from "./dmTimeline";

const base = (id: string): Message => ({
  id,
  channelId: "dm",
  author: { id: "u-ada", displayName: "Ada" },
  content: id,
  attachments: [],
  embeds: [],
  createdAt: "2026-11-18T10:00:00Z",
});
const sealed = (id: string, reason: SealedReason = "pin"): Message => ({ ...base(id), content: "", sealed: { reason, count: 1 } });
const line = (id: string, kind: TimelineLineKind, actorId = "u-ada"): Message => ({ ...base(id), content: "", line: { kind, actorId } });

describe("sealed runs", () => {
  it("become one entry per run that counts them, split where the reason changes", () => {
    const collapse = createSealedCollapser();
    const shown = collapse([base("a"), sealed("s1"), sealed("s2"), sealed("s3", "lost"), base("b"), sealed("s4")]);
    expect(shown.map((message) => [message.id, message.sealed?.count ?? 0])).toEqual([
      ["a", 0],
      ["s1", 2],
      ["s3", 1],
      ["b", 0],
      ["s4", 1],
    ]);
  });

  it("keep the same entry while the run does not change, so the transcript does not redraw it", () => {
    const collapse = createSealedCollapser();
    const messages = [sealed("s1"), sealed("s2")];
    expect(collapse(messages)[0]).toBe(collapse([...messages])[0]);
  });
});

describe("encryption lines", () => {
  const requested = (by: string): ConversationEncryption => ({
    state: "requested",
    requestedById: by,
    requestedAt: "x",
    enabledAt: null,
    currentEpochId: null,
  });

  it("know which request still waits for an answer", () => {
    const history = [line("r1", "encryption_requested"), line("c1", "encryption_request_cancelled"), line("r2", "encryption_requested")];
    expect(encryptionLineFacts(history, requested("u-ada")).liveRequestId).toBe("r2");
    expect(encryptionLineFacts(history, { ...requested("u-ada"), state: "off" }).liveRequestId).toBeNull();
    expect(encryptionLineFacts(history.slice(0, 2), requested("u-ada")).liveRequestId).toBeNull();
  });

  it("tell a declined request from encryption switched off", () => {
    const facts = encryptionLineFacts(
      [
        line("r1", "encryption_requested"),
        line("d1", "encryption_disabled", "u-me"),
        line("r2", "encryption_requested"),
        line("e2", "encryption_enabled", "u-me"),
        line("d2", "encryption_disabled"),
      ],
      undefined,
    );
    expect([...facts.declinedIds]).toEqual(["d1"]);
  });
});
