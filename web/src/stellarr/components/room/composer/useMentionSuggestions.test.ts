import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { MentionCandidate } from "@/lib/messages/composerMentions";

import { useMentionSuggestions } from "./useMentionSuggestions";

const people: MentionCandidate[] = [
  { id: "u-old", username: "everyone", displayName: "Old account" },
  { id: "u-ewa", username: "ewa", displayName: "Ewa Nowak" },
];
const groups: MentionCandidate[] = [
  { id: "group:everyone", username: "everyone", displayName: "Notify everyone in this channel", group: "everyone" },
];

describe("useMentionSuggestions", () => {
  it("offers nobody under a group's word in the text, where writing it calls on the group", () => {
    const { result } = renderHook(() =>
      useMentionSuggestions({ text: "@e", candidates: people, groups, locale: "en", enabled: true }));
    act(() => result.current.track(2));

    expect(result.current.matches.map((candidate) => candidate.id)).toEqual(["group:everyone", "u-ewa"]);
  });

  it("still lets a command's person argument pick an account that took the word before it was reserved", () => {
    const { result } = renderHook(() =>
      useMentionSuggestions({ text: "e", candidates: people, locale: "en", enabled: true, whole: true }));

    expect(result.current.matches.map((candidate) => candidate.id)).toEqual(["u-old", "u-ewa"]);
  });
});
