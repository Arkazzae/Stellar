import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import type { PersonVM } from "@/components/view-models";
import {
  layOutRail,
  moveTo,
  pressAt,
  release,
  ROW,
  stubPointerCapture,
  TEXT_TOP,
  translateOf,
  VOICE_TOP,
} from "@/test/railLayout";

import { Rail, type RailProps } from "./Rail";

/*
 * The drags run for real here, on the layout `layOutRail` stands in for:
 * text channels from y = 100 in 32 px rows, then the voice channels from
 * y = 400, Salon (with Jo) 62 px tall and Sesja below it.
 */

const jo: PersonVM = {
  id: "u-2",
  displayName: "Jo",
  initials: "JO",
  presence: "online",
};

const SALON_TOP = VOICE_TOP;
const SESJA_TOP = VOICE_TOP + ROW + 30;
const JO_MIDDLE = SALON_TOP + ROW + 15;

function setup(overrides: Partial<RailProps> = {}) {
  const props: RailProps = {
    serverName: "Orbit",
    textChannels: [
      { id: "t-1", name: "ogólny" },
      { id: "t-2", name: "plany" },
      { id: "t-3", name: "luzem" },
    ],
    voiceChannels: [
      { id: "v-1", name: "Salon", occupants: [jo] },
      { id: "v-2", name: "Sesja", occupants: [] },
    ],
    activeChannelId: null,
    selfId: "me",
    onSelectTextChannel: vi.fn(),
    onJoinVoiceChannel: vi.fn(),
    onEditServer: vi.fn(),
    onInvite: vi.fn(),
    onReorderChannelGroup: vi.fn(),
    ...overrides,
  };
  const view = (next: RailProps) => (
    <ConfirmProvider>
      <Rail {...next} />
    </ConfirmProvider>
  );
  const { rerender } = render(view(props));
  return { ...props, rerender: (next: Partial<RailProps>) => rerender(view({ ...props, ...next })) };
}

const channel = (name: string) => screen.getByRole("button", { name });
const occupant = () =>
  screen.getByRole("button", { name: "Jo, przenieś do innego kanału głosowego" });
const zone = (name: string) =>
  [...document.querySelectorAll(".voice-channel-drop")].find(
    (element) => element.querySelector(".channel .name")?.textContent === name,
  );
const centreOf = (index: number) => TEXT_TOP + index * ROW + ROW / 2;

/** Carries the text channel at `from` onto the slot at `to` with the mouse. */
function dragText(name: string, from: number, to: number) {
  pressAt(channel(name), centreOf(from));
  moveTo(centreOf(from) + (to > from ? 8 : -8), centreOf(to));
  release(centreOf(to));
  fireEvent.click(channel(name));
}

/** Carries Jo from Salon to the height `y` with the mouse and lets go. */
function dragJoTo(y: number) {
  pressAt(occupant(), JO_MIDDLE);
  moveTo(JO_MIDDLE + 8, y);
  release(y);
  fireEvent.click(document.body);
}

beforeEach(() => {
  layOutRail();
  stubPointerCapture();
});

describe("Rail text channel order", () => {
  it("reports the whole group in its new order after a drop", () => {
    const { onReorderChannelGroup } = setup();

    dragText("luzem", 2, 0);

    expect(onReorderChannelGroup).toHaveBeenCalledWith(["t-3", "t-1", "t-2"]);
  });

  it("lands a channel dragged downwards on the slot it was dropped on", () => {
    const { onReorderChannelGroup } = setup();

    dragText("ogólny", 0, 2);

    expect(onReorderChannelGroup).toHaveBeenCalledWith(["t-2", "t-3", "t-1"]);
  });

  it("reports nothing when a channel is dropped back on its own slot", () => {
    const { onReorderChannelGroup, onSelectTextChannel } = setup();

    pressAt(channel("plany"), centreOf(1));
    moveTo(centreOf(1) + 10, centreOf(1) + 4);
    release(centreOf(1) + 4);
    fireEvent.click(channel("plany"));

    expect(onReorderChannelGroup).not.toHaveBeenCalled();
    expect(onSelectTextChannel).not.toHaveBeenCalled();
  });

  it("says the order was not saved, then clears the notice on the next try", async () => {
    const onReorderChannelGroup = vi
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(undefined);
    setup({ onReorderChannelGroup });

    dragText("luzem", 2, 0);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nie udało się zapisać kolejności kanałów.",
    );

    dragText("plany", 1, 0);
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });
});

describe("Rail voice moves", () => {
  it("asks for the person to be moved to the channel they were dropped on", async () => {
    const onMoveVoiceMember = vi.fn(async () => {});
    setup({ onMoveVoiceMember });

    pressAt(occupant(), JO_MIDDLE);
    moveTo(JO_MIDDLE + 8, SESJA_TOP + 10);
    expect(zone("Sesja")).toHaveClass("is-over");
    expect(zone("Salon")).not.toHaveClass("is-over");
    expect(occupant()).toHaveClass("is-dragging");
    const copy = document.querySelector<HTMLElement>(".voice-occupant-carry");
    expect(copy).toHaveTextContent("Jo");
    expect(copy?.style.top).toBe(`${SALON_TOP + ROW}px`);
    expect(translateOf(copy)).toBe(`translateY(${SESJA_TOP + 10 - JO_MIDDLE}px)`);
    release(SESJA_TOP + 10);
    fireEvent.click(document.body);

    expect(onMoveVoiceMember).toHaveBeenCalledWith("u-2", "v-1", "v-2");
    expect(document.querySelector(".voice-occupant-carry")).toBeNull();
    expect(screen.getByText("Upuszczono użytkownika Jo na kanale Sesja.")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByText("Przeniesiono użytkownika Jo.")).toBeInTheDocument(),
    );
  });

  it("targets the channel at the copy's height even when the pointer leaves the rail sideways", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    pressAt(occupant(), JO_MIDDLE);
    fireEvent.pointerMove(document, {
      pointerId: 1,
      isPrimary: true,
      clientX: 900,
      clientY: SESJA_TOP + 10,
    });
    expect(zone("Sesja")).toHaveClass("is-over");
    release(SESJA_TOP + 10);
    fireEvent.click(document.body);

    expect(onMoveVoiceMember).toHaveBeenCalledWith("u-2", "v-1", "v-2");
  });

  it("moves nobody when released outside every channel", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    dragJoTo(SESJA_TOP + 300);

    expect(onMoveVoiceMember).not.toHaveBeenCalled();
    expect(screen.getByText("Anulowano przenoszenie.")).toBeInTheDocument();
  });

  it("moves a person with the keyboard, one channel per arrow", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    fireEvent.keyDown(occupant(), { key: " " });
    expect(screen.getByText("Podniesiono użytkownika Jo.")).toBeInTheDocument();
    expect(zone("Salon")).toHaveClass("is-over");

    fireEvent.keyDown(occupant(), { key: "ArrowDown" });
    expect(zone("Sesja")).toHaveClass("is-over");
    expect(screen.getByText("Nad kanałem Sesja.")).toBeInTheDocument();
    // The last channel is as far as the arrows go.
    fireEvent.keyDown(occupant(), { key: "ArrowDown" });
    expect(zone("Sesja")).toHaveClass("is-over");

    fireEvent.keyDown(occupant(), { key: " " });
    expect(onMoveVoiceMember).toHaveBeenCalledWith("u-2", "v-1", "v-2");
  });

  it("puts the person back on Escape", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    fireEvent.keyDown(occupant(), { key: "Enter" });
    fireEvent.keyDown(occupant(), { key: "ArrowDown" });
    fireEvent.keyDown(occupant(), { key: "Escape" });

    expect(onMoveVoiceMember).not.toHaveBeenCalled();
    expect(zone("Sesja")).not.toHaveClass("is-over");
    expect(screen.getByText("Anulowano przenoszenie.")).toBeInTheDocument();
  });

  it("drops the drag when the person leaves the call mid-way", () => {
    const onMoveVoiceMember = vi.fn();
    const { rerender } = setup({ onMoveVoiceMember });

    pressAt(occupant(), JO_MIDDLE);
    moveTo(JO_MIDDLE + 8, SESJA_TOP + 10);
    rerender({
      voiceChannels: [
        { id: "v-1", name: "Salon", occupants: [] },
        { id: "v-2", name: "Sesja", occupants: [] },
      ],
    });

    expect(document.querySelector(".voice-occupant-carry")).toBeNull();
    expect(zone("Sesja")).not.toHaveClass("is-over");
    release(SESJA_TOP + 10);
    fireEvent.click(document.body);
    expect(onMoveVoiceMember).not.toHaveBeenCalled();
  });

  it("marks the person as moving until the server answers", async () => {
    let finish: () => void = () => {};
    setup({
      onMoveVoiceMember: () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    });

    dragJoTo(SESJA_TOP + 10);

    expect(screen.getByText("Jo").closest("li")).toHaveAttribute("aria-busy", "true");
    expect(screen.getByText("Przenoszenie użytkownika Jo.")).toBeInTheDocument();

    await act(async () => finish());
    expect(screen.getByText("Jo").closest("li")).not.toHaveAttribute("aria-busy");
  });

  it("reports a failed move as an alert", async () => {
    setup({
      onMoveVoiceMember: vi.fn().mockRejectedValue(new Error("forbidden")),
    });

    dragJoTo(SESJA_TOP + 10);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nie udało się przenieść użytkownika Jo.",
    );
  });

  it("does nothing for a drop on the channel the person is already in", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    dragJoTo(SALON_TOP + 10);

    expect(onMoveVoiceMember).not.toHaveBeenCalled();
  });
});
