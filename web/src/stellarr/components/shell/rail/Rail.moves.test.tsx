import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import type {
  ChannelMove,
  PersonVM,
  TextChannelVM,
  VoiceChannelVM,
} from "@/components/view-models";
import {
  layOutRail,
  moveTo,
  nextFrame,
  pressAt,
  railTop,
  release,
  ROW,
  stubPointerCapture,
  translateOf,
} from "@/test/railLayout";

import { Rail, type RailProps } from "./Rail";

/*
 * The drags run for real here, on the layout `layOutRail` stands in for: the
 * rail stacked from y = 0 in its content, 22 px captions, 32 px rows and
 * 30 px people. With everything open:
 *
 *     0  TEXT          22 ogólny  54 plany  86 luzem
 *   118  VOICE        140 Salon (with Jo, 62 px)  202 Sesja
 *   234  Projekty     256 zadania  288 notatki  320 Warsztat
 *   352  Archiwum     374 (empty: no channels, a note)
 */

const jo: PersonVM = { id: "u-2", displayName: "Jo", initials: "JO", presence: "online" };

const TEXT: TextChannelVM[] = [
  { id: "t-1", name: "ogólny" },
  { id: "t-2", name: "plany" },
  { id: "t-3", name: "luzem" },
  { id: "t-4", name: "zadania", sectionId: "sec-1" },
  { id: "t-5", name: "notatki", sectionId: "sec-1" },
];
const VOICE: VoiceChannelVM[] = [
  { id: "v-1", name: "Salon", occupants: [jo] },
  { id: "v-2", name: "Sesja", occupants: [] },
  { id: "v-3", name: "Warsztat", occupants: [], sectionId: "sec-1" },
];

/** `channels` with a move applied as the directory applies it: the channel's section, and its group's order. */
function applyMove<T extends TextChannelVM | VoiceChannelVM>(channels: T[], move: ChannelMove): T[] {
  if (!channels.some((item) => item.id === move.channelId)) return channels;
  const placed = channels.map((item) =>
    item.id === move.channelId ? { ...item, sectionId: move.sectionId ?? undefined } : item,
  );
  const group = new Set(move.order);
  let next = 0;
  return placed.map((item) => {
    if (!group.has(item.id)) return item;
    const id = move.order[next++];
    return placed.find((other) => other.id === id) ?? item;
  });
}

/** The rail over channels that take its moves, as the directory store would. */
function LiveRail(props: RailProps) {
  const fresh = (from: RailProps) => ({
    from,
    text: from.textChannels,
    voice: from.voiceChannels,
    sections: from.sections ?? [],
  });
  const [shown, setShown] = useState(() => fresh(props));
  if (shown.from !== props) setShown(fresh(props));
  const { onMoveSection } = props;
  return (
    <Rail
      {...props}
      textChannels={shown.text}
      voiceChannels={shown.voice}
      sections={shown.sections}
      {...(onMoveSection === undefined
        ? {}
        : {
            onMoveSection: (order: string[]) => {
              setShown((current) => ({
                ...current,
                sections: order.flatMap((id) => current.sections.filter((section) => section.id === id)),
              }));
              return onMoveSection(order);
            },
          })}
      onMoveChannel={(move) => {
        setShown((current) => ({
          ...current,
          text: applyMove(current.text, move),
          voice: applyMove(current.voice, move),
        }));
        return props.onMoveChannel(move);
      }}
    />
  );
}

function setup(overrides: Partial<RailProps> = {}) {
  const props: RailProps = {
    serverName: "Orbit",
    textChannels: TEXT,
    voiceChannels: VOICE,
    sections: [
      { id: "sec-1", name: "Projekty" },
      { id: "sec-2", name: "Archiwum" },
    ],
    activeChannelId: null,
    selfId: "me",
    canManageChannels: true,
    onSelectTextChannel: vi.fn(),
    onJoinVoiceChannel: vi.fn(),
    onEditServer: vi.fn(),
    onInvite: vi.fn(),
    onMoveChannel: vi.fn(),
    ...overrides,
  };
  const view = (next: RailProps) => (
    <ConfirmProvider>
      <LiveRail {...next} />
    </ConfirmProvider>
  );
  const { rerender } = render(view(props));
  return { ...props, rerender: (next: Partial<RailProps>) => rerender(view({ ...props, ...next })) };
}

/** A channel's main button; a voice channel's name runs on with how many are in the call. */
const channel = (name: string) =>
  screen.getByRole("button", {
    name: (accessible) => accessible === name || new RegExp(`^${name}\\d`).test(accessible),
  });
const said = () =>
  [...document.querySelectorAll('[aria-live="assertive"]')]
    .map((node) => node.textContent)
    .filter(Boolean)
    .join(" ");
const sortHint = () => document.getElementById(channel("ogólny").getAttribute("aria-describedby") ?? "");

/** Presses a channel's row in its middle and carries it until its top is held at `top`. */
function carry(name: string, top: number) {
  const row = channel(name).closest(".text-channel-node, .voice-channel-drop")!;
  const middle = railTop(row) + ROW / 2;
  pressAt(channel(name), middle);
  moveTo(middle + 8, middle + top - railTop(row));
}

/** Carries a channel until its top is held at `top` and lets go there. */
function drop(name: string, top: number) {
  carry(name, top);
  const row = channel(name).closest(".text-channel-node, .voice-channel-drop")!;
  release(railTop(row) + ROW / 2 + top - railTop(row));
  // The click the browser sends after the release must not open the channel.
  fireEvent.click(channel(name));
}

/** Lets go at `y`, with the click a browser sends after it, which the drag eats. */
function letGo(y: number) {
  release(y);
  fireEvent.click(document.body);
}

beforeEach(() => {
  layOutRail();
  stubPointerCapture();
});

describe("Rail channel sorting", () => {
  it("hides empty default captions while keeping channel creation and moves out of sections", () => {
    const onCreateChannel = vi.fn();
    const { onMoveChannel } = setup({
      textChannels: TEXT.filter((item) => item.sectionId !== undefined),
      voiceChannels: VOICE.filter((item) => item.sectionId !== undefined),
      onCreateChannel,
    });

    expect(screen.queryByRole("button", { name: /^tekstowe$/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /^głosowe$/i })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Utwórz kanał" }));
    expect(onCreateChannel).toHaveBeenCalledExactlyOnceWith("text");

    const tasks = channel("zadania");
    tasks.focus();
    fireEvent.keyDown(tasks, { key: " " });
    fireEvent.keyDown(tasks, { key: "ArrowUp" });
    fireEvent.keyDown(tasks, { key: " " });
    expect(onMoveChannel).toHaveBeenCalledExactlyOnceWith({
      channelId: "t-4",
      sectionId: null,
      order: ["t-4"],
    });
    expect(screen.getByRole("button", { name: /^tekstowe$/i })).toBeVisible();
    expect(screen.queryByRole("button", { name: /^głosowe$/i })).toBeNull();
    expect(channel("zadania")).toHaveFocus();
  });

  it("sends the group's new order when a channel is dropped in its own group", () => {
    const { onMoveChannel, onSelectTextChannel } = setup();

    // ogólny (22) lands where luzem ends up once it moves up a row: 54.
    drop("ogólny", 54);

    expect(onMoveChannel).toHaveBeenCalledExactlyOnceWith({
      channelId: "t-1",
      sectionId: null,
      order: ["t-2", "t-1", "t-3"],
    });
    expect(onSelectTextChannel).not.toHaveBeenCalled();
    expect(said()).toBe("Upuszczono kanał ogólny na pozycji 2 z 3 w grupie Tekstowe.");
  });

  it("moves a channel into another section with one call that carries both section and order", async () => {
    const { onMoveChannel } = setup();

    // Between zadania and notatki: notatki starts at 288, and with ogólny
    // taken out of the rail above it, 32 px higher.
    carry("ogólny", 256);
    await nextFrame();
    const row = (name: string) => channel(name).closest(".text-channel-node");
    // The carried row follows the pointer; the loose group closes up behind
    // it, and the section makes room in front of it.
    expect(translateOf(row("ogólny"))).toBe("translateY(234px)");
    expect(translateOf(row("plany"))).toBe("translateY(-32px)");
    expect(translateOf(row("notatki"))).toBe("");
    expect(said()).toBe("Kanał ogólny, pozycja 2 z 3 w grupie Projekty.");
    letGo(272);

    expect(onMoveChannel).toHaveBeenCalledExactlyOnceWith({
      channelId: "t-1",
      sectionId: "sec-1",
      order: ["t-4", "t-1", "t-5"],
    });
    expect(translateOf(row("plany"))).toBe("");
  });

  it("drops at the end of a collapsed section, and into an empty one", () => {
    const { onMoveChannel } = setup();

    fireEvent.click(screen.getByRole("button", { name: /Projekty/ }));
    // Right below the caption of the collapsed Projekty, which ends at 256
    // (224 with ogólny taken out).
    drop("ogólny", 224);
    expect(onMoveChannel).toHaveBeenLastCalledWith({
      channelId: "t-1",
      sectionId: "sec-1",
      order: ["t-4", "t-5", "t-1"],
    });

    // With ogólny gone and Projekty closed, Archiwum's empty text list is
    // at 246; 214 with plany taken out.
    drop("plany", 214);
    expect(onMoveChannel).toHaveBeenLastCalledWith({
      channelId: "t-2",
      sectionId: "sec-2",
      order: ["t-2"],
    });
  });

  it("walks the arrows from one group into the next and keeps focus on the dropped channel", () => {
    const { onMoveChannel } = setup();
    const luzem = channel("luzem");
    luzem.focus();

    fireEvent.keyDown(luzem, { key: " " });
    expect(said()).toBe("Podniesiono kanał luzem, pozycja 3 z 3 w grupie Tekstowe.");
    // luzem is last in its group; the next place down is the top of Projekty.
    fireEvent.keyDown(luzem, { key: "ArrowDown" });
    expect(said()).toBe("Kanał luzem, pozycja 1 z 3 w grupie Projekty.");
    fireEvent.keyDown(luzem, { key: " " });

    expect(onMoveChannel).toHaveBeenCalledExactlyOnceWith({
      channelId: "t-3",
      sectionId: "sec-1",
      order: ["t-3", "t-4", "t-5"],
    });
    // The row was rendered anew under the section; focus went with it.
    expect(channel("luzem")).toHaveFocus();
  });

  it("sorts a voice channel with its people, among voice channels only", async () => {
    const { onMoveChannel } = setup();
    const salon = () => channel("Salon").closest(".voice-channel-drop");

    // The end of Projekty's voice list is at 352; 290 with Salon (62 px) out.
    carry("Salon", 290);
    await nextFrame();
    expect(translateOf(salon())).toBe("translateY(150px)");
    expect(salon()).toHaveTextContent("Jo");
    letGo(156 + 150);

    expect(onMoveChannel).toHaveBeenCalledExactlyOnceWith({
      channelId: "v-1",
      sectionId: "sec-1",
      order: ["v-3", "v-1"],
    });
  });

  it("leaves a press that does not travel 6 px a click that joins the call", () => {
    const { onMoveChannel, onJoinVoiceChannel } = setup();

    pressAt(channel("Sesja"), 218);
    moveTo(221, 223);
    release(223);
    fireEvent.click(channel("Sesja"));

    expect(onJoinVoiceChannel).toHaveBeenCalledExactlyOnceWith("v-2");
    expect(onMoveChannel).not.toHaveBeenCalled();
  });

  it("offers no sorting to whoever may not edit channels", () => {
    const { onMoveChannel, onSelectTextChannel } = setup({ canManageChannels: false });

    expect(channel("ogólny")).not.toHaveAttribute("aria-roledescription");
    expect(channel("Salon")).not.toHaveAttribute("aria-roledescription");
    expect(sortHint()).toBeNull();
    pressAt(channel("ogólny"), 38);
    moveTo(46, 120);
    release(120);
    fireEvent.click(channel("ogólny"));

    expect(onMoveChannel).not.toHaveBeenCalled();
    expect(onSelectTextChannel).toHaveBeenCalledExactlyOnceWith("t-1");
  });

  it("says the order was not saved, then clears the notice on the next try", async () => {
    const onMoveChannel = vi
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(undefined);
    setup({ onMoveChannel });

    drop("luzem", 22);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nie udało się zapisać kolejności kanałów.",
    );

    drop("plany", 22);
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("ends a drag cleanly when someone else reorders the channels under it", () => {
    const { onMoveChannel, rerender } = setup();

    carry("ogólny", 54);
    rerender({ textChannels: [TEXT[1]!, TEXT[0]!, ...TEXT.slice(2)] });

    expect(said()).toBe("Anulowano przestawianie kanału.");
    release(90);
    fireEvent.click(channel("ogólny"));
    expect(onMoveChannel).not.toHaveBeenCalled();
  });

  it("measures again and carries on when only a call's people change under it", () => {
    const { onMoveChannel, rerender } = setup();

    carry("Sesja", 290);
    // Jo leaves Salon: everything below it moves up 30 px.
    rerender({ voiceChannels: [{ ...VOICE[0]!, occupants: [] }, ...VOICE.slice(1)] });
    letGo(218 + 290 - 202);

    expect(onMoveChannel).toHaveBeenCalledExactlyOnceWith({
      channelId: "v-2",
      sectionId: "sec-1",
      order: ["v-3", "v-2"],
    });
  });
});

describe("Rail section sorting", () => {
  const caption = (name: string) => screen.getByRole("button", { name });

  it("moves a section by its caption, below the loose groups, and a click still closes it", () => {
    const onMoveSection = vi.fn();
    setup({ onMoveSection });

    // Archiwum (352) goes above Projekty (234): its caption is pressed 17 px
    // into the section, and its top is held where Projekty's is.
    pressAt(caption("Archiwum"), 369);
    moveTo(377, 251);
    letGo(251);
    expect(onMoveSection).toHaveBeenCalledExactlyOnceWith(["sec-2", "sec-1"]);
    expect(said()).toBe("Upuszczono sekcję Archiwum na pozycji 1 z 2.");

    fireEvent.click(caption("Projekty"));
    expect(caption("Projekty")).toHaveAttribute("aria-expanded", "false");
  });

  it("walks a section down with the keyboard and keeps focus on its caption", () => {
    const onMoveSection = vi.fn();
    setup({ onMoveSection });
    const projekty = caption("Projekty");
    projekty.focus();

    fireEvent.keyDown(projekty, { key: " " });
    expect(said()).toBe("Podniesiono sekcję Projekty, pozycja 1 z 2.");
    fireEvent.keyDown(projekty, { key: "ArrowDown" });
    fireEvent.keyDown(projekty, { key: " " });

    expect(onMoveSection).toHaveBeenCalledExactlyOnceWith(["sec-2", "sec-1"]);
    expect(caption("Projekty")).toHaveFocus();
  });

  it("leaves the captions plain buttons for whoever may not reorder sections", () => {
    const { onMoveChannel } = setup();

    expect(caption("Projekty")).not.toHaveAttribute("aria-roledescription");
    pressAt(caption("Archiwum"), 369);
    moveTo(377, 251);
    letGo(251);
    expect(onMoveChannel).not.toHaveBeenCalled();
  });
});

describe("Rail voice moves", () => {
  /** Jo's row, under Salon's 32 px button. */
  const JO_MIDDLE = 140 + ROW + 15;
  const SESJA = 202;
  const occupant = () =>
    screen.getByRole("button", { name: "Jo, przenieś do innego kanału głosowego" });
  const zone = (name: string) => channel(name).closest(".voice-channel-drop");

  function dragJoTo(y: number) {
    pressAt(occupant(), JO_MIDDLE);
    moveTo(JO_MIDDLE + 8, y);
    release(y);
    fireEvent.click(document.body);
  }

  it("asks for the person to be moved to the channel they were dropped on, in any section", async () => {
    const onMoveVoiceMember = vi.fn(async () => {});
    setup({ onMoveVoiceMember });

    // Warsztat sits in Projekty, 320 px down the rail.
    pressAt(occupant(), JO_MIDDLE);
    moveTo(JO_MIDDLE + 8, 330);
    await nextFrame();
    expect(zone("Warsztat")).toHaveAttribute("data-drop-over");
    expect(zone("Salon")).not.toHaveAttribute("data-drop-over");
    expect(document.querySelector(".voice-occupant-carry")).toHaveTextContent("Jo");
    letGo(330);

    expect(onMoveVoiceMember).toHaveBeenCalledExactlyOnceWith("u-2", "v-1", "v-3");
    expect(document.querySelector(".voice-occupant-carry")).toBeNull();
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
      clientY: SESJA + 10,
    });
    letGo(SESJA + 10);

    expect(onMoveVoiceMember).toHaveBeenCalledWith("u-2", "v-1", "v-2");
  });

  it("moves nobody when released outside every channel", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    dragJoTo(SESJA + 600);

    expect(onMoveVoiceMember).not.toHaveBeenCalled();
    expect(screen.getByText("Anulowano przenoszenie.")).toBeInTheDocument();
  });

  it("puts the person back on Escape", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    fireEvent.keyDown(occupant(), { key: "Enter" });
    fireEvent.keyDown(occupant(), { key: "ArrowDown" });
    fireEvent.keyDown(occupant(), { key: "Escape" });

    expect(onMoveVoiceMember).not.toHaveBeenCalled();
    expect(zone("Sesja")).not.toHaveAttribute("data-drop-over");
  });

  it("moves a person with the keyboard, one channel per arrow across the sections", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    fireEvent.keyDown(occupant(), { key: " " });
    fireEvent.keyDown(occupant(), { key: "ArrowDown" });
    expect(zone("Sesja")).toHaveAttribute("data-drop-over");
    fireEvent.keyDown(occupant(), { key: "ArrowDown" });
    expect(said()).toBe("Nad kanałem Warsztat.");
    // Projekty's Warsztat is the last voice channel; the arrows stop there.
    fireEvent.keyDown(occupant(), { key: "ArrowDown" });
    fireEvent.keyDown(occupant(), { key: " " });

    expect(onMoveVoiceMember).toHaveBeenCalledExactlyOnceWith("u-2", "v-1", "v-3");
  });

  it("drops the drag when the person leaves the call mid-way", () => {
    const onMoveVoiceMember = vi.fn();
    const { rerender } = setup({ onMoveVoiceMember });

    pressAt(occupant(), JO_MIDDLE);
    moveTo(JO_MIDDLE + 8, SESJA + 10);
    rerender({ voiceChannels: [{ ...VOICE[0]!, occupants: [] }, ...VOICE.slice(1)] });

    expect(document.querySelector(".voice-occupant-carry")).toBeNull();
    release(SESJA + 10);
    fireEvent.click(document.body);
    expect(onMoveVoiceMember).not.toHaveBeenCalled();
  });

  it("marks the person as moving until the server answers, and reports a failure", async () => {
    let fail: (error: Error) => void = () => {};
    setup({
      onMoveVoiceMember: () =>
        new Promise<void>((_, reject) => {
          fail = reject;
        }),
    });

    dragJoTo(SESJA + 10);
    expect(screen.getByText("Jo").closest("li")).toHaveAttribute("aria-busy", "true");

    await act(async () => fail(new Error("forbidden")));
    expect(screen.getByText("Jo").closest("li")).not.toHaveAttribute("aria-busy");
    expect(screen.getByRole("alert")).toHaveTextContent("Nie udało się przenieść użytkownika Jo.");
  });

  it("does nothing for a drop on the channel the person is already in", () => {
    const onMoveVoiceMember = vi.fn();
    setup({ onMoveVoiceMember });

    dragJoTo(140 + 10);

    expect(onMoveVoiceMember).not.toHaveBeenCalled();
  });
});

describe("Rail: carrying yourself", () => {
  const maja: PersonVM = { id: "me", displayName: "Maja", initials: "MA", presence: "online" };
  /** You in Salon instead of Jo, so the rail keeps the layout drawn at the top. */
  const SEATED: VoiceChannelVM[] = [{ ...VOICE[0]!, occupants: [maja] }, ...VOICE.slice(1)];
  const MAJA_MIDDLE = 140 + ROW + 15;
  const self = () =>
    screen.getByRole("button", { name: "Maja, przenieś do innego kanału głosowego" });
  const zone = (name: string) => channel(name).closest(".voice-channel-drop")!;

  function dragSelfTo(y: number) {
    pressAt(self(), MAJA_MIDDLE);
    moveTo(MAJA_MIDDLE + 8, y);
    letGo(y);
  }

  it("joins the channel you drop yourself on, as a click would, and never asks for a staff move", () => {
    const onJoinVoiceChannel = vi.fn();
    const onMoveVoiceMember = vi.fn();
    setup({ voiceChannels: SEATED, onJoinVoiceChannel, onMoveVoiceMember });

    // Warsztat sits in Projekty, 320 px down the rail.
    dragSelfTo(330);

    expect(onJoinVoiceChannel).toHaveBeenCalledExactlyOnceWith("v-3");
    expect(onMoveVoiceMember).not.toHaveBeenCalled();
    expect(said()).toBe("Przenosimy Cię na kanał Warsztat.");
    expect(screen.getByText("Maja").closest("li")).not.toHaveAttribute("aria-busy");
  });

  it("lets you carry yourself without the right to move others, whose rows stay plain", () => {
    const onJoinVoiceChannel = vi.fn();
    setup({
      voiceChannels: [{ ...VOICE[0]!, occupants: [maja, jo] }, ...VOICE.slice(1)],
      onJoinVoiceChannel,
    });

    const others = screen.getByText("Jo").closest("li");
    expect(others).not.toHaveAttribute("role");
    expect(others).not.toHaveAttribute("aria-roledescription");

    fireEvent.keyDown(self(), { key: " " });
    expect(said()).toBe("Podniesiono Cię.");
    fireEvent.keyDown(self(), { key: "ArrowDown" });
    fireEvent.keyDown(self(), { key: " " });
    expect(onJoinVoiceChannel).toHaveBeenCalledExactlyOnceWith("v-2");
  });

  it("does nothing for a drop back on your own channel, or for a cancel", () => {
    const onJoinVoiceChannel = vi.fn();
    setup({ voiceChannels: SEATED, onJoinVoiceChannel });

    dragSelfTo(140 + 10);
    expect(said()).toBe("Anulowano przenoszenie.");
    fireEvent.keyDown(self(), { key: " " });
    fireEvent.keyDown(self(), { key: "ArrowDown" });
    fireEvent.keyDown(self(), { key: "Escape" });

    expect(onJoinVoiceChannel).not.toHaveBeenCalled();
  });

  it("passes over a full channel, which you could not join by clicking either", async () => {
    const onJoinVoiceChannel = vi.fn();
    setup({
      voiceChannels: [
        SEATED[0]!,
        { ...VOICE[1]!, occupants: [jo], userLimit: 1 },
        { ...VOICE[2]!, userLimit: 2 },
      ],
      onJoinVoiceChannel,
    });

    const sesja = railTop(zone("Sesja"));
    pressAt(self(), MAJA_MIDDLE);
    moveTo(MAJA_MIDDLE + 8, sesja + 10);
    await nextFrame();
    expect(zone("Sesja")).not.toHaveAttribute("data-drop-over");
    letGo(sesja + 10);
    expect(onJoinVoiceChannel).not.toHaveBeenCalled();

    fireEvent.keyDown(self(), { key: " " });
    fireEvent.keyDown(self(), { key: "ArrowDown" });
    expect(said()).toBe("Nad kanałem Warsztat.");
    fireEvent.keyDown(self(), { key: " " });
    expect(onJoinVoiceChannel).toHaveBeenCalledExactlyOnceWith("v-3");
  });

  it("stops targeting a channel that fills up while you are carried over it", () => {
    const onJoinVoiceChannel = vi.fn();
    const sesja = { ...VOICE[1]!, userLimit: 1 };
    const { rerender } = setup({
      voiceChannels: [SEATED[0]!, sesja, VOICE[2]!],
      onJoinVoiceChannel,
    });

    fireEvent.keyDown(self(), { key: " " });
    fireEvent.keyDown(self(), { key: "ArrowDown" });
    expect(zone("Sesja")).toHaveAttribute("data-drop-over");
    rerender({ voiceChannels: [SEATED[0]!, { ...sesja, occupants: [jo] }, VOICE[2]!] });
    expect(zone("Sesja")).not.toHaveAttribute("data-drop-over");
    fireEvent.keyDown(self(), { key: " " });

    expect(onJoinVoiceChannel).not.toHaveBeenCalled();
  });
});
