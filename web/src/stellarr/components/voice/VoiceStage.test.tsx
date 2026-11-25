import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { PersonVM, ScreenShareVM } from "@/components/view-models";

import { VoiceStage, type VoiceStageProps } from "./VoiceStage";

const ADA_SCREEN: ScreenShareVM = {
  key: "remote:ada:s1",
  ownerId: "ada",
  ownerName: "Ada",
  local: false,
  hasAudio: true,
  stream: null,
  watching: false,
};

const BO_SCREEN: ScreenShareVM = {
  key: "remote:bo:s2",
  ownerId: "bo",
  ownerName: "Bo",
  local: false,
  hasAudio: false,
  stream: null,
  watching: false,
};

const OWN_SCREEN: ScreenShareVM = {
  key: "local:me:s0",
  ownerId: "me",
  ownerName: "Me",
  local: true,
  hasAudio: false,
  stream: {} as MediaStream,
  watching: false,
};

const PEOPLE: PersonVM[] = [
  { id: "ada", displayName: "Ada", initials: "AD", presence: "online", speaking: true },
  { id: "bo", displayName: "Bo", initials: "BO", presence: "online" },
];

function stage(props: Partial<VoiceStageProps> = {}) {
  return (
    <VoiceStage
      people={PEOPLE}
      shares={[]}
      onWatchShare={vi.fn()}
      onStopWatchingShare={vi.fn()}
      {...props}
    />
  );
}

const root = (container: HTMLElement) => container.firstElementChild as HTMLElement;
const focusedTile = (container: HTMLElement) => container.querySelector<HTMLElement>(".tile.is-focus");

beforeEach(() => window.localStorage.clear());
afterEach(() => vi.useRealTimers());

describe("VoiceStage", () => {
  it("gives every shared screen a tile of its own beside the people", () => {
    const { container } = render(stage({ shares: [ADA_SCREEN, BO_SCREEN] }));

    expect(root(container)).toHaveAttribute("data-layout", "grid");
    expect(screen.getByRole("button", { name: "Oglądaj: Ada (ekran)" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Oglądaj: Bo (ekran)" })).toBeVisible();
    expect(container.querySelectorAll(".tile")).toHaveLength(4);
    // There is no surface switch above the stage any more.
    expect(screen.queryByRole("group", { name: "Co pokazać na scenie" })).toBeNull();
  });

  it("watches a screen and gives it the stage in one click", async () => {
    const user = userEvent.setup();
    const onWatchShare = vi.fn();
    const { container } = render(stage({ shares: [ADA_SCREEN, BO_SCREEN], onWatchShare }));

    await user.click(screen.getByRole("button", { name: "Oglądaj: Bo (ekran)" }));

    expect(onWatchShare).toHaveBeenCalledWith("remote:bo:s2");
    expect(root(container)).toHaveAttribute("data-layout", "focus");
    expect(focusedTile(container)).toHaveAttribute("data-key", "remote:bo:s2");
  });

  it("goes back to the grid without stopping, or stops watching and goes back", async () => {
    const user = userEvent.setup();
    const onStopWatchingShare = vi.fn();
    const watched = { ...ADA_SCREEN, watching: true };
    const { container, rerender } = render(stage({ shares: [ADA_SCREEN], onStopWatchingShare }));

    await user.click(screen.getByRole("button", { name: "Oglądaj: Ada (ekran)" }));
    rerender(stage({ shares: [watched], onStopWatchingShare }));
    expect(root(container)).toHaveAttribute("data-layout", "focus");

    await user.click(within(focusedTile(container)!).getByRole("button", { name: "Siatka" }));
    expect(root(container)).toHaveAttribute("data-layout", "grid");
    expect(onStopWatchingShare).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Pokaż na scenie: Ada (ekran)" }));
    expect(root(container)).toHaveAttribute("data-layout", "focus");

    await user.click(screen.getByRole("button", { name: "Przestań oglądać" }));
    expect(onStopWatchingShare).toHaveBeenCalledOnce();
    expect(root(container)).toHaveAttribute("data-layout", "grid");
  });

  it("gives anyone the stage on a click and offers the way back to the grid", async () => {
    const user = userEvent.setup();
    const { container } = render(stage());

    await user.click(screen.getByRole("button", { name: "Pokaż na scenie: Bo" }));
    expect(root(container)).toHaveAttribute("data-layout", "focus");
    expect(focusedTile(container)).toHaveTextContent("Bo");

    await user.click(screen.getByRole("button", { name: "Siatka" }));
    expect(root(container)).toHaveAttribute("data-layout", "grid");
  });

  it("hides the strip of people, remembers it on this device, and shows who speaks", async () => {
    const user = userEvent.setup();
    const { container, unmount } = render(stage());

    await user.click(screen.getByRole("button", { name: "Pokaż na scenie: Bo" }));
    await user.click(screen.getByRole("button", { name: "Schowaj osoby" }));

    expect(root(container)).toHaveAttribute("data-strip", "hidden");
    expect(screen.getByRole("button", { name: /Pokaż osoby/ })).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelector(".stage-face.speaking")).not.toBeNull();
    unmount();

    const again = render(stage());
    expect(root(again.container)).toHaveAttribute("data-strip", "hidden");
  });

  it("keeps your own screen a tile with its preview, which you can hide, and stops it there", async () => {
    const user = userEvent.setup();
    const onStopSharing = vi.fn();
    const { container } = render(stage({ shares: [OWN_SCREEN], onStopSharing }));

    expect(root(container)).toHaveAttribute("data-layout", "grid");
    expect(screen.getByText("Twój ekran", { selector: ".st-label span" })).toBeTruthy();
    expect(container.querySelector(".tile.screen video")).not.toBeNull();

    await user.click(screen.getByRole("button", { name: "Ukryj podgląd" }));
    expect(container.querySelector(".tile.screen video")).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("Podgląd ukryty. Inni nadal widzą Twój ekran.");

    await user.click(screen.getByRole("button", { name: "Pokaż podgląd" }));
    expect(container.querySelector(".tile.screen video")).not.toBeNull();

    await user.click(screen.getByRole("button", { name: "Zatrzymaj ekran" }));
    expect(onStopSharing).toHaveBeenCalledOnce();
  });

  it("routes the sharer's volume to the person sharing, and says when a screen has no sound", async () => {
    const user = userEvent.setup();
    const onParticipantVolumeChange = vi.fn();
    const props = {
      participantVolumes: new Map([["ada", 125]]),
      onParticipantVolumeChange,
    };
    const { rerender } = render(stage({ shares: [ADA_SCREEN, BO_SCREEN], ...props }));
    await user.click(screen.getByRole("button", { name: "Oglądaj: Ada (ekran)" }));
    rerender(stage({ shares: [{ ...ADA_SCREEN, watching: true }, BO_SCREEN], ...props }));

    const slider = screen.getByRole("slider", { name: "Głośność: Ada" });
    expect(slider).toHaveValue("125");
    fireEvent.change(slider, { target: { value: "70" } });
    expect(onParticipantVolumeChange).toHaveBeenCalledWith("ada", 70);

    rerender(stage({ shares: [ADA_SCREEN, { ...BO_SCREEN, watching: true }], ...props }));
    expect(screen.getByText("Bez dźwięku")).toBeTruthy();
  });

  it("says why a watched screen has no picture", () => {
    const { rerender } = render(stage({ shares: [{ ...ADA_SCREEN, watching: true, connecting: true }] }));
    expect(screen.getByRole("status")).toHaveTextContent("Łączę z ekranem");

    rerender(stage({ shares: [{ ...ADA_SCREEN, watching: true, heldBack: true }] }));
    expect(screen.getByRole("status")).toHaveTextContent("Obraz wstrzymany: za słabe łącze");

    rerender(stage({ shares: [{ ...ADA_SCREEN, watching: true, sealed: true }] }));
    expect(screen.getByRole("status")).toHaveTextContent("Nie można odszyfrować obrazu od Ada");
  });

  it("says a screen share ended for a moment, then lets its tile go", () => {
    vi.useFakeTimers();
    const { container, rerender } = render(stage({ shares: [ADA_SCREEN] }));

    rerender(stage({ shares: [] }));
    expect(screen.getByRole("status")).toHaveTextContent("Udostępnianie zakończone");
    expect(container.querySelectorAll(".tile.screen")).toHaveLength(1);

    act(() => vi.advanceTimersByTime(3000));
    expect(container.querySelectorAll(".tile.screen")).toHaveLength(0);
  });

  it("marks the stage for cinema", () => {
    const { container } = render(stage({ cinema: true }));
    expect(root(container)).toHaveClass("voice-stage", "cinema");
  });
});
