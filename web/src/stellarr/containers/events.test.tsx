import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { eventWindow } from "@/lib/events/eventSheetRequest";
import type { ServerEvent } from "@/ports";
import { serverEvent } from "@/test/events";
import { polish } from "@/test/translators";

import { EventCardContainer } from "./EventCardContainer";
import { EventWindowHost } from "./EventWindowHost";
import { EventsRailContainer } from "./EventsRailContainer";

const world = vi.hoisted(() => ({
  events: new Map<string, unknown>(),
  listeners: new Set<() => void>(),
  directory: { canCreateEvents: false, canManageEvents: false },
  userId: "u-reader",
  serverId: "srv-1",
  join: vi.fn(),
  signUp: vi.fn(async () => {}),
}));

const NONE: ServerEvent[] = [];

const port = {
  list: () => {
    const coming = [...world.events.values()] as ServerEvent[];
    return coming.length === 0 ? NONE : coming;
  },
  get: (id: string) => world.events.get(id) as ServerEvent | undefined,
  subscribe: (listener: () => void) => {
    world.listeners.add(listener);
    return () => world.listeners.delete(listener);
  },
  load: vi.fn(async () => {}),
  remember: (event: ServerEvent) => {
    const known = world.events.get(event.id) as ServerEvent | undefined;
    if (known === undefined || known.revision < event.revision) world.events.set(event.id, event);
  },
  signUp: world.signUp,
  signOut: vi.fn(async () => {}),
  people: vi.fn(async () => ({ people: [], nextCursor: null })),
  cancel: vi.fn(async () => {}),
};

vi.mock("@/state/hooks", () => ({
  usePorts: () => ({ events: port }),
  useDirectory: () => ({
    server: { id: world.serverId, name: "Stellarr Dev" },
    textChannels: [],
    voiceChannels: [{ id: "vc-gaming", name: "Gaming", occupants: [] }],
    members: [],
    ...world.directory,
  }),
  useSession: () => ({ user: { id: world.userId } }),
  useUi: () => ({ joinVoiceChannel: world.join }),
  useLanguage: () => ({ locale: "pl", t: polish }),
}));

/** Someone else's change, heard live: a newer copy, and its listeners told. */
function heard(event: ServerEvent) {
  act(() => {
    world.events.set(event.id, event);
    for (const listener of [...world.listeners]) listener();
  });
}

beforeEach(() => {
  world.events.clear();
  world.listeners.clear();
  world.directory = { canCreateEvents: false, canManageEvents: false };
  world.userId = "u-reader";
  world.serverId = "srv-1";
  vi.clearAllMocks();
  port.load.mockImplementation(async () => {});
  eventWindow.close();
});

const renderCard = (snapshot: ServerEvent) =>
  render(<EventCardContainer snapshot={snapshot} />, { wrapper: ConfirmProvider });

describe("an announced event's card", () => {
  it("shows the copy it came with until a newer one is heard, and signs up for the current date", async () => {
    const user = userEvent.setup();
    const future = { startsAt: "2999-01-01T19:00:00Z", endsAt: "2999-01-01T21:00:00Z" };
    renderCard(serverEvent({ ...future, revision: 3 }));
    expect(screen.getByText("7 z 10 miejsc zajętych")).toBeInTheDocument();

    heard(serverEvent({ ...future, revision: 4, signedUp: 9 }));
    expect(screen.getByText("9 z 10 miejsc zajętych")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Zapisz się" }));
    expect(world.signUp).toHaveBeenCalledWith(expect.objectContaining({ revision: 4, occurrenceId: "occ-1" }));
  });

  it("joins its voice channel the way the rail does", async () => {
    const user = userEvent.setup();
    renderCard(serverEvent({ state: "live", startsAt: "2020-01-01T19:00:00Z", endsAt: undefined }));

    await user.click(screen.getByRole("button", { name: "Dołącz do Gaming" }));

    expect(world.join).toHaveBeenCalledExactlyOnceWith("vc-gaming");
  });

  it.each([
    ["its creator", () => (world.userId = "u-kenji"), true],
    ["whoever manages events", () => (world.directory = { ...world.directory, canManageEvents: true }), true],
    ["anyone else", () => undefined, false],
  ])("offers changing and cancelling to %s only", (_who, arrange, offered) => {
    arrange();
    renderCard(serverEvent({ startsAt: "2999-01-01T19:00:00Z", endsAt: undefined }));
    expect(screen.queryByRole("button", { name: "Zarządzaj wydarzeniem" }) !== null).toBe(offered);
  });
});

describe("the events row", () => {
  it("is not there with nothing coming and nothing to create", () => {
    const { container } = render(<EventsRailContainer />);
    expect(container).toBeEmptyDOMElement();
  });

  it("offers a new event to whoever may create one", async () => {
    const user = userEvent.setup();
    world.directory = { ...world.directory, canCreateEvents: true };
    render(<EventsRailContainer />);

    await user.click(screen.getByRole("button", { name: /Wydarzenia/ }));
    await user.click(within(screen.getByRole("dialog", { name: "Wydarzenia" })).getByRole("button", { name: "Nowe" }));

    expect(eventWindow.get()).toEqual({ kind: "create" });
  });
});

describe("an event opened from a notice", () => {
  it("is looked for in its server's events once that server is on screen", async () => {
    world.serverId = "srv-other";
    port.load.mockImplementation(async () => {
      world.events.set("ev-1", serverEvent({ startsAt: "2999-01-01T19:00:00Z", endsAt: undefined }));
    });
    const view = render(<EventWindowHost />, { wrapper: ConfirmProvider });
    act(() => eventWindow.open({ kind: "view", eventId: "ev-1", serverId: "srv-1" }));
    // Still on another server: nothing is read for it yet.
    expect(port.load).not.toHaveBeenCalled();

    world.serverId = "srv-1";
    view.rerender(<EventWindowHost />);

    expect(await screen.findByRole("article", { name: "Wieczór planszówek" })).toBeInTheDocument();
    expect(port.load).toHaveBeenCalledWith("srv-1");
  });

  it("says plainly when the event is not among them", async () => {
    render(<EventWindowHost />, { wrapper: ConfirmProvider });
    act(() => eventWindow.open({ kind: "view", eventId: "ev-gone", serverId: "srv-1" }));

    expect(await screen.findByRole("dialog", { name: "Tego wydarzenia już nie ma" })).toHaveTextContent(
      "Skończyło się, zostało odwołane albo nie jest już dla Ciebie widoczne.",
    );
  });
});
