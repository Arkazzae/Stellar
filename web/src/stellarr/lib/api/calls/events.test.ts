import { describe, expect, it } from "vitest";

import { EventDraftRefusedError, EventRefusedError } from "@/ports/events";
import { serverEvent } from "@/test/events";
import { jsonResponse, mockFetch, sentRequest } from "@/test/http";

import { parseMessage } from "../chatParsers";
import { changedBody, eventBody, eventCalls, parseEvent, parseEventSignupChanged } from "./events";

const fetchMock = mockFetch();

/** An event as the server writes it: every missing value a null. */
const wire = (extra: Record<string, unknown> = {}) => ({
  id: "ev-1",
  serverId: "srv-1",
  name: "Wieczór planszówek",
  description: null,
  place: { kind: "voice", channelId: "vc-gaming" },
  timeZone: "Europe/Warsaw",
  repeat: "weekly",
  seats: 10,
  createdBy: "u-kenji",
  announceMessageId: null,
  occurrenceId: "occ-1",
  startsAt: "2026-11-28T19:00:00Z",
  endsAt: null,
  state: "upcoming",
  liveSince: null,
  endedAt: null,
  signedUp: 7,
  waiting: 0,
  faces: [{ id: "u-marta", displayName: "Marta" }],
  mine: "signed",
  myPosition: null,
  revision: 12,
  ...extra,
});

const problem = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/problem+json" } });

const draft = {
  name: " Wieczór planszówek ",
  place: { kind: "voice" as const, channelId: "vc-gaming" },
  startsAt: new Date("2026-11-28T19:00:00Z"),
  timeZone: "Europe/Warsaw",
  repeat: "none" as const,
};

describe("an event from the server", () => {
  it("leaves out what is null, and keeps the reader's place only where it is known", () => {
    const read = parseEvent(wire(), "event");
    expect(read).toMatchObject({ id: "ev-1", occurrenceId: "occ-1", mine: "signed", faces: [{ id: "u-marta", displayName: "Marta" }] });
    expect(read).not.toHaveProperty("endsAt");
    expect(read).not.toHaveProperty("myPosition");
    expect(parseEvent(wire({ mine: null }), "event")).not.toHaveProperty("mine");
    expect(parseEvent(wire({ mine: "waiting", myPosition: 3 }), "event").myPosition).toBe(3);
  });

  it("shows an announcement without an event it cannot read", () => {
    const message = parseMessage({
      id: "m1",
      channelId: "c1",
      author: { id: "u1", displayName: "Kenji" },
      content: "",
      event: { id: "ev-1", state: "someday" },
    });
    expect(message).not.toHaveProperty("event");
    expect(parseMessage({ id: "m2", channelId: "c1", author: { id: "u1" }, content: "", event: wire() }).event?.id).toBe(
      "ev-1",
    );
  });

  it("reads the reader's own place, and passes over one it cannot read", () => {
    expect(parseEventSignupChanged({ eventId: "ev-1", occurrenceId: "occ-1", revision: 3, mine: "waiting", myPosition: 2 })).toEqual({
      eventId: "ev-1",
      occurrenceId: "occ-1",
      revision: 3,
      mine: "waiting",
      myPosition: 2,
    });
    expect(parseEventSignupChanged({ eventId: "ev-1", revision: 3, mine: "signed" })).toBeNull();
  });
});

describe("sending an event", () => {
  it("sends every member, trimmed, with what is absent as null", () => {
    expect(eventBody(draft)).toEqual({
      name: "Wieczór planszówek",
      description: null,
      place: { kind: "voice", channelId: "vc-gaming" },
      startsAt: "2026-11-28T19:00:00.000Z",
      endsAt: null,
      timeZone: "Europe/Warsaw",
      seats: null,
      repeat: "none",
      announceChannelId: null,
    });
  });

  it("changes only what was changed, so a running event's name changes without its time", () => {
    const running = serverEvent({ state: "live", endsAt: undefined, seats: undefined, description: undefined });
    expect(changedBody(running, { ...draft, name: "Wieczór gier" })).toEqual({ name: "Wieczór gier" });
  });

  it("puts the server's refusals on the sheet's fields, and says why a state refused", async () => {
    fetchMock.mockResolvedValueOnce(problem(400, { errors: { "place.text": ["Too long."], announceChannelId: ["Hidden."] } }));
    const refused = eventCalls().createEvent(draft);
    await expect(refused).rejects.toBeInstanceOf(EventDraftRefusedError);
    await expect(refused).rejects.toMatchObject({ fields: ["place", "announce"] });

    fetchMock.mockResolvedValueOnce(problem(409, { code: "event-limit" }));
    await expect(eventCalls().createEvent(draft)).rejects.toMatchObject({ reason: "limit" });
  });
});

describe("signing up", () => {
  it("signs up and out of the current date, and answers with the event", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse(wire()));
    fetchMock.mockResolvedValueOnce(jsonResponse(wire({ mine: "none", signedUp: 6 })));

    expect((await eventCalls().signUp("ev-1", "occ-1")).mine).toBe("signed");
    expect((await eventCalls().signOut("ev-1", "occ-1")).signedUp).toBe(6);

    expect(sentRequest(fetchMock, 0)).toMatchObject({ method: "PUT", url: "/api/events/ev-1/occurrences/occ-1/signup" });
    expect(sentRequest(fetchMock, 1)).toMatchObject({ method: "DELETE", url: "/api/events/ev-1/occurrences/occ-1/signup" });
  });

  it("says a date has moved on, or the event is over", async () => {
    fetchMock.mockResolvedValueOnce(problem(409, { code: "event-occurrence-gone" }));
    const moved = eventCalls().signUp("ev-1", "occ-1");
    await expect(moved).rejects.toBeInstanceOf(EventRefusedError);
    await expect(moved).rejects.toMatchObject({ reason: "moved" });

    fetchMock.mockResolvedValueOnce(problem(409, { code: "event-over" }));
    await expect(eventCalls().signUp("ev-1", "occ-1")).rejects.toMatchObject({ reason: "over" });
  });

  it("reads who signed up, seated first, then the waitlist", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        people: [
          { user: { id: "u1", displayName: "Kenji" }, state: "signed", position: null },
          { user: { id: "u2", displayName: "Marta" }, state: "waiting", position: 1 },
        ],
        nextCursor: null,
      }),
    );

    const page = await eventCalls().people("ev-1", "occ-1");

    expect(sentRequest(fetchMock).url).toBe("/api/events/ev-1/occurrences/occ-1/signups?limit=100");
    expect(page.people).toEqual([
      { id: "u1", displayName: "Kenji", state: "signed" },
      { id: "u2", displayName: "Marta", state: "waiting", position: 1 },
    ]);
  });
});
