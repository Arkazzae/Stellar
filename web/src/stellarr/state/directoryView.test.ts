import { describe, expect, it } from "vitest";

import type { Channel, User, VoiceOccupant } from "@/ports";

import {
  createDirectoryViewDeriver,
  deriveDirectoryView,
  type DirectorySnapshot,
  sameOccupancy,
  serverMark,
  spliceGroupOrder,
} from "./directoryView";

const channel = (id: string): Channel => ({ id, name: id, kind: "text" });
const occupant = (userId: string): VoiceOccupant => ({ userId, displayName: userId, muted: false });

describe("spliceGroupOrder", () => {
  const channels = ["a", "b", "c", "d", "e"].map(channel);

  it("puts the new order into the slots the group already held", () => {
    expect(spliceGroupOrder(channels, ["d", "b"])).toEqual(["a", "d", "c", "b", "e"]);
  });

  it("leaves the order alone when the group keeps its sequence", () => {
    expect(spliceGroupOrder(channels, ["b", "d"])).toEqual(["a", "b", "c", "d", "e"]);
  });
});

describe("sameOccupancy", () => {
  it("treats two empty lists as the same answer", () => {
    expect(sameOccupancy(new Map([["v", []]]), new Map([["v", []]]))).toBe(true);
  });

  it("sees a person arrive", () => {
    expect(sameOccupancy(new Map([["v", []]]), new Map([["v", [occupant("ada")]]]))).toBe(false);
  });

  it("sees a different set of channels", () => {
    expect(sameOccupancy(new Map([["v", []]]), new Map([["w", []]]))).toBe(false);
    expect(sameOccupancy(new Map(), new Map([["v", []]]))).toBe(false);
  });

  it("is satisfied by the very same list", () => {
    const list = [occupant("ada")];

    expect(sameOccupancy(new Map([["v", list]]), new Map([["v", list]]))).toBe(true);
  });
});

describe("serverMark", () => {
  it.each([
    ["Orbit", "OR"],
    ["Nocna zmiana", "NZ"],
    ["  spaced   out  ", "SO"],
    ["", "ST"],
  ])("marks %j as %s", (name, mark) => {
    expect(serverMark(name)).toBe(mark);
  });
});

describe("deriveDirectoryView", () => {
  it("keeps a voice channel's seat limit, which decides who can join it, and none for 0", () => {
    const view = deriveDirectoryView({
      server: { id: "s", name: "Orbit" },
      servers: [],
      channels: [
        { id: "small", name: "small", kind: "voice", userLimit: 2 },
        { id: "open", name: "open", kind: "voice", userLimit: 0 },
      ],
      sections: [],
      members: [],
      permissions: [],
      occupancy: new Map([["small", [occupant("ada")]]]),
    });

    expect(view.voiceChannels.map(({ id, userLimit }) => ({ id, userLimit }))).toEqual([
      { id: "small", userLimit: 2 },
      { id: "open", userLimit: undefined },
    ]);
  });
});

describe("createDirectoryViewDeriver", () => {
  const user = (id: string): User => ({ id, username: id, displayName: id, presence: "online" });
  const snapshot = (patch: Partial<DirectorySnapshot>): DirectorySnapshot => ({
    server: { id: "s", name: "Orbit" },
    servers: [],
    channels: [
      { id: "lounge", name: "lounge", kind: "voice" },
      { id: "studio", name: "studio", kind: "voice" },
    ],
    sections: [],
    members: [],
    permissions: [],
    occupancy: new Map(),
    ...patch,
  });

  it("hands back whatever says the same as before, and anew only what changed", () => {
    const derive = createDirectoryViewDeriver();
    const ada = user("ada");
    const bo = user("bo");
    const first = derive(
      snapshot({
        members: [ada, bo],
        occupancy: new Map([
          ["lounge", [occupant("ada")]],
          ["studio", [occupant("bo")]],
        ]),
      }),
    );

    // A presence change replaces one member; a hub frame brings fresh objects
    // for a room in which nothing changed, and a mute in the other one.
    const next = derive(
      snapshot({
        members: [ada, { ...bo, presence: "idle" }],
        occupancy: new Map([
          ["lounge", [occupant("ada")]],
          ["studio", [{ ...occupant("bo"), muted: true }]],
        ]),
      }),
    );

    expect(next.people[0]).toBe(first.people[0]);
    expect(next.people[1]).toEqual({ ...first.people[1], presence: "idle" });
    expect(next.voiceChannels[0]).toBe(first.voiceChannels[0]);
    expect(next.voiceChannels[1]?.occupants[0]?.muted).toBe(true);
    expect(next.occupantsOf("lounge")).toBe(first.occupantsOf("lounge"));

    const quiet = derive(
      snapshot({
        members: next.members,
        occupancy: new Map([
          ["lounge", [occupant("ada")]],
          ["studio", [{ ...occupant("bo"), muted: true }]],
        ]),
      }),
    );
    expect(quiet.voiceChannels).toBe(next.voiceChannels);
    expect(quiet.people).toBe(next.people);
    expect(quiet.inVoice).toBe(next.inVoice);
    expect(quiet.occupantsOf).toBe(next.occupantsOf);
  });
});
