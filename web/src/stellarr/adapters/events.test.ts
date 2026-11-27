import { describe, expect, it, vi } from "vitest";

import type { EventApiClient } from "@/lib/api/calls/events";
import { serverEvent } from "@/test/events";
import { FakeRealtimeClient } from "@/test/fakes";

import { createEventsAdapter } from "./events";

function setup(first = [serverEvent()]) {
  const api = {
    listEvents: vi.fn<EventApiClient["listEvents"]>(async () => first),
    getEvent: vi.fn<EventApiClient["getEvent"]>(),
    createEvent: vi.fn<EventApiClient["createEvent"]>(async () => serverEvent({ id: "new", startsAt: "2026-11-27T19:00:00Z" })),
    editEvent: vi.fn<EventApiClient["editEvent"]>(),
    cancelEvent: vi.fn<EventApiClient["cancelEvent"]>(async () => {}),
    signUp: vi.fn<EventApiClient["signUp"]>(async () => serverEvent({ mine: "signed", signedUp: 8, revision: 2 })),
    signOut: vi.fn<EventApiClient["signOut"]>(),
    people: vi.fn<EventApiClient["people"]>(),
  };
  const realtime = new FakeRealtimeClient();
  const port = createEventsAdapter(api, realtime);
  const names = () => port.list("srv-1").map((event) => event.id);
  return { api, realtime, port, names };
}

describe("the events port", () => {
  it("lists the server's coming events soonest first, and takes a new one in", async () => {
    const { port, names } = setup([serverEvent({ id: "later", startsAt: "2026-12-01T19:00:00Z" }), serverEvent()]);
    await port.load("srv-1");
    expect(names()).toEqual(["ev-1", "later"]);

    await port.create({ name: "x", place: { kind: "elsewhere", text: "x" }, startsAt: new Date(), timeZone: "UTC", repeat: "none" });
    expect(names()).toEqual(["new", "ev-1", "later"]);
  });

  it("signs up for the current date and shows the answer at once", async () => {
    const { api, port } = setup();
    await port.load("srv-1");

    await port.signUp(port.list("srv-1")[0]!);

    expect(api.signUp).toHaveBeenCalledWith("ev-1", "occ-1");
    expect(port.get("ev-1")).toMatchObject({ mine: "signed", signedUp: 8 });
  });

  it("follows live changes: counts for everyone, and the reader's own place for them", async () => {
    const { realtime, port } = setup();
    await port.load("srv-1");

    const { mine: _mine, ...shared } = serverEvent({ revision: 3, signedUp: 10, waiting: 1 });
    realtime.push("EventChanged", shared);
    realtime.push("EventSignupChanged", { eventId: "ev-1", occurrenceId: "occ-1", revision: 4, mine: "waiting", myPosition: 1 });

    expect(port.get("ev-1")).toMatchObject({ signedUp: 10, mine: "waiting", myPosition: 1 });
  });

  it("drops one that ended or was cancelled from the list, while its card can still read it", async () => {
    const { realtime, port, names } = setup();
    await port.load("srv-1");

    realtime.push("EventChanged", serverEvent({ state: "cancelled", revision: 5 }));

    expect(names()).toEqual([]);
    expect(port.get("ev-1")?.state).toBe("cancelled");
  });

  it("forgets a coming one the next read leaves out, and reads again after a reconnect", async () => {
    const { api, realtime, port, names } = setup();
    await port.load("srv-1");
    api.listEvents.mockResolvedValue([]);

    realtime.drop();
    await realtime.start();
    await vi.waitFor(() => expect(api.listEvents).toHaveBeenCalledTimes(2));

    await vi.waitFor(() => expect(names()).toEqual([]));
  });
});
