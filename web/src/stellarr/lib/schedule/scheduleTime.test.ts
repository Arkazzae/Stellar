import { describe, expect, it } from "vitest";

import { parseWhen, schedulePresets, timeProblem } from "./scheduleTime";
import { instantAt } from "./zonedTime";

const WARSAW = "Europe/Warsaw";
const NEW_YORK = "America/New_York";
/** Thursday 26 November 2026, 16:30 in Warsaw. */
const THURSDAY = new Date("2026-11-26T15:30:00Z");

const presets = (now: Date, zone: string) =>
  schedulePresets(now, zone).map((preset) => [preset.id, preset.at.toISOString()]);
const at = (input: string, locale: "pl" | "en" | "de" = "pl", now = THURSDAY) => {
  const parsed = parseWhen(input, now, WARSAW, locale);
  return "at" in parsed ? parsed.at.toISOString() : parsed.problem;
};

describe("the send-later presets", () => {
  it("offer this evening while there is time to write, tomorrow morning and next Monday morning", () => {
    expect(presets(THURSDAY, WARSAW)).toEqual([
      ["today", "2026-11-26T17:00:00.000Z"],
      ["tomorrow", "2026-11-27T08:00:00.000Z"],
      ["monday", "2026-11-30T08:00:00.000Z"],
    ]);
    // An hour before the evening, there is no evening left to offer.
    expect(presets(new Date("2026-11-26T16:30:00Z"), WARSAW).map(([id]) => id)).toEqual(["tomorrow", "monday"]);
  });

  it("skip next Monday when tomorrow is Monday, and a Monday's Monday is a week away", () => {
    const sunday = new Date("2026-11-29T09:00:00Z");
    expect(presets(sunday, WARSAW).map(([id]) => id)).toEqual(["today", "tomorrow"]);
    const monday = new Date("2026-11-30T09:00:00Z");
    expect(presets(monday, WARSAW)).toContainEqual(["monday", "2026-12-07T08:00:00.000Z"]);
  });

  it("are read in the reader's zone, not the machine's", () => {
    // The same instant is 10:30 in New York.
    expect(presets(THURSDAY, NEW_YORK)).toEqual([
      ["today", "2026-11-26T23:00:00.000Z"],
      ["tomorrow", "2026-11-27T14:00:00.000Z"],
      ["monday", "2026-11-30T14:00:00.000Z"],
    ]);
  });
});

describe("the send-time limits", () => {
  it("allow from a minute ahead to thirty days ahead", () => {
    const later = (ms: number) => new Date(THURSDAY.getTime() + ms);
    expect(timeProblem(later(-1), THURSDAY)).toBe("past");
    expect(timeProblem(later(59_000), THURSDAY)).toBe("tooSoon");
    expect(timeProblem(later(60_000), THURSDAY)).toBeNull();
    expect(timeProblem(later(30 * 86_400_000), THURSDAY)).toBeNull();
    expect(timeProblem(later(30 * 86_400_000 + 1), THURSDAY)).toBe("tooFar");
  });

  it("keep a clock time across a change of season", () => {
    // 02:30 does not happen on the spring night: the next real time is 03:30.
    expect(instantAt({ year: 2026, month: 3, day: 29 }, { hour: 2, minute: 30 }, WARSAW).toISOString()).toBe(
      "2026-03-29T01:30:00.000Z",
    );
    // It happens twice on the autumn night: the first one.
    expect(instantAt({ year: 2026, month: 10, day: 25 }, { hour: 2, minute: 30 }, WARSAW).toISOString()).toBe(
      "2026-10-25T00:30:00.000Z",
    );
  });
});

describe("reading the time typed after /zaplanuj", () => {
  it("reads a day and a time in Polish, with or without its accents", () => {
    expect(at("jutro 9:00")).toBe("2026-11-27T08:00:00.000Z");
    expect(at("jutro o 9")).toBe("2026-11-27T08:00:00.000Z");
    expect(at("pon 18:00")).toBe("2026-11-30T17:00:00.000Z");
    expect(at("dziś 18:00")).toBe("2026-11-26T17:00:00.000Z");
    expect(at("dzis 18:00")).toBe("2026-11-26T17:00:00.000Z");
    expect(at("środa 12:15")).toBe("2026-12-02T11:15:00.000Z");
  });

  it("reads an amount of time ahead", () => {
    expect(at("za 2 h")).toBe("2026-11-26T17:30:00.000Z");
    expect(at("za 30 min")).toBe("2026-11-26T16:00:00.000Z");
    expect(at("in 2h", "en")).toBe("2026-11-26T17:30:00.000Z");
    expect(at("in 2 std", "de")).toBe("2026-11-26T17:30:00.000Z");
  });

  it("reads English in every language, and each language's own words only in it", () => {
    expect(at("tomorrow 9:00", "pl")).toBe("2026-11-27T08:00:00.000Z");
    expect(at("mon 6pm", "en")).toBe("2026-11-30T17:00:00.000Z");
    expect(at("morgen 9:00", "de")).toBe("2026-11-27T08:00:00.000Z");
    expect(at("mo 18:00", "de")).toBe("2026-11-30T17:00:00.000Z");
    expect(at("tomorrow 9:00", "de")).toBe("2026-11-27T08:00:00.000Z");
    expect(at("jutro 9:00", "de")).toBe("unknown");
    expect(at("morgen 9:00", "en")).toBe("unknown");
  });

  it("takes a time alone as today, or tomorrow once it has passed, and a weekday as the next one ahead", () => {
    expect(at("18:00")).toBe("2026-11-26T17:00:00.000Z");
    expect(at("10:00")).toBe("2026-11-27T09:00:00.000Z");
    // Thursday at 10:00 has passed this Thursday: next week's.
    expect(at("czw 10:00")).toBe("2026-12-03T09:00:00.000Z");
    expect(at("czw 18:00")).toBe("2026-11-26T17:00:00.000Z");
    expect(at("jutro")).toBe("2026-11-27T08:00:00.000Z");
  });

  it("says why a time cannot be used", () => {
    expect(at("dziś 10:00")).toBe("past");
    expect(at("za 800 h")).toBe("tooFar");
    expect(at("kiedyś")).toBe("unknown");
    expect(at("jutro 25:00")).toBe("unknown");
    expect(at("")).toBe("unknown");
  });
});
