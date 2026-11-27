import { describe, expect, it } from "vitest";

import { polish } from "@/test/translators";
import { BOARD_CHANNELS, serverEvent } from "@/test/events";

import { eventAction, eventListLine, eventPhase, eventStatus, eventWhen, placeLink, railSummary } from "./eventCard";

const ZONE = "Europe/Warsaw";
/** Thursday 26 November 2026, 13:30 in Warsaw. */
const BEFORE = new Date("2026-11-26T12:30:00Z");
/** Saturday, 20:10 in Warsaw: the game night has begun. */
const DURING = new Date("2026-11-28T19:10:00Z");
const AFTER = new Date("2026-11-28T21:30:00Z");

const GAMING = BOARD_CHANNELS["vc-gaming"];

const card = (extra: Parameters<typeof serverEvent>[0], now = BEFORE, channel = {}) => {
  const item = serverEvent(extra);
  return { status: eventStatus(item, now, "pl", polish, channel), action: eventAction(item, now, channel).kind };
};

describe("an event's card", () => {
  it.each([
    ["seats left", {}, "7 z 10 miejsc zajętych", "signUp"],
    ["signed up", { mine: "signed" as const, signedUp: 8 }, "8 z 10 · przypomnienie 15 min przed", "signedUp"],
    ["full", { signedUp: 10, waiting: 2 }, "Komplet · 2 osoby czekają", "joinWaitlist"],
    ["on the waitlist", { signedUp: 10, waiting: 3, mine: "waiting" as const, myPosition: 3 }, "Jesteś 3. w kolejce", "onWaitlist"],
    ["no limit", { seats: undefined, signedUp: 5 }, "5 osób zapisanych", "signUp"],
    ["nobody yet", { signedUp: 0 }, "Nikt się jeszcze nie zapisał", "signUp"],
  ])("says how it stands and offers one thing: %s", (_case, extra, status, action) => {
    expect(card(extra)).toEqual({ status, action });
  });

  it("offers its voice channel to everyone while it runs, signed up or not, full or not", () => {
    const channel = { ...GAMING, present: 5, signedAbsent: 3 };
    expect(card({}, DURING, channel)).toEqual({ status: "3 zapisane osoby jeszcze nie dołączyły", action: "join" });
    expect(card({ signedUp: 10, waiting: 4, mine: "waiting", myPosition: 2 }, DURING, channel).action).toBe("join");
    expect(eventAction(serverEvent(), DURING, GAMING)).toEqual({ kind: "join", channelId: "vc-gaming", channelName: "Gaming" });
    // Somewhere else, there is nothing to join from here.
    expect(card({ place: { kind: "elsewhere", text: "Kawiarnia" } }, DURING).action).toBe("none");
  });

  it("runs from its start and is over at its end, whatever the server has said so far", () => {
    expect(eventPhase(serverEvent(), BEFORE)).toBe("upcoming");
    expect(eventPhase(serverEvent(), DURING)).toBe("live");
    expect(eventPhase(serverEvent(), AFTER)).toBe("ended");
    // A voice channel's event without an end runs until the server ends it.
    expect(eventPhase(serverEvent({ endsAt: undefined }), AFTER)).toBe("live");
    expect(card({}, AFTER)).toEqual({ status: "7 osób zapisanych", action: "none" });
  });

  it("says when, in the reader's zone, and how long it has been running", () => {
    expect(eventWhen(serverEvent(), BEFORE, ZONE, "pl", polish)).toBe("sobota, 28 listopada · 20:00–22:00");
    expect(eventWhen(serverEvent({ endsAt: undefined, repeat: "weekly" }), BEFORE, ZONE, "pl", polish)).toBe(
      "sobota, 28 listopada · 20:00 · co tydzień",
    );
    expect(eventWhen(serverEvent({ liveSince: "2026-11-28T19:04:00Z" }), DURING, ZONE, "pl", polish, { present: 5 })).toBe(
      "od 20:04 · 5 osób na kanale",
    );
    expect(eventWhen(serverEvent(), BEFORE, "America/New_York", "pl", polish)).toBe("sobota, 28 listopada · 14:00–16:00");
  });

  it("offers nothing once cancelled, and says nothing of seats", () => {
    expect(card({ state: "cancelled" })).toEqual({ status: "", action: "none" });
  });

  it("links a place only when it is a web address", () => {
    expect(placeLink(serverEvent({ place: { kind: "elsewhere", text: "https://meet.example.org/abc" } }))).toBe(
      "https://meet.example.org/abc",
    );
    expect(placeLink(serverEvent({ place: { kind: "elsewhere", text: "javascript:alert(1)" } }))).toBeUndefined();
    expect(placeLink(serverEvent({ place: { kind: "elsewhere", text: "Kawiarnia Kafka" } }))).toBeUndefined();
  });
});

describe("the list and the rail", () => {
  it("sums up a date in one line, with how it repeats first", () => {
    expect(eventListLine(serverEvent({ signedUp: 8 }), ZONE, "pl", polish, GAMING)).toBe("20:00 · Gaming · 8 z 10 miejsc");
    expect(
      eventListLine(
        serverEvent({ repeat: "weekly", seats: undefined, signedUp: 4, place: { kind: "voice", channelId: "vc-meet" } }),
        ZONE,
        "pl",
        polish,
        BOARD_CHANNELS["vc-meet"],
      ),
    ).toBe("co tydzień · 20:00 · Spotkania · 4 osoby");
  });

  it("counts what is coming and says when one runs", () => {
    const coming = serverEvent({ id: "a" });
    const later = serverEvent({ id: "b", startsAt: "2026-11-30T09:00:00Z", endsAt: undefined });
    expect(railSummary([coming, later], BEFORE)).toEqual({ upcoming: 2, live: false });
    expect(railSummary([coming, later], DURING)).toEqual({ upcoming: 1, live: true });
  });
});
