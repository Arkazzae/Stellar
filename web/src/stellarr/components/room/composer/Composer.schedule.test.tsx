import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ScheduleRefusedError } from "@/ports";

import { Composer, type ComposerProps, type ComposerScheduling } from "./Composer";

/** Thursday 26 November 2026, 16:30 in Warsaw, as on the board. */
const THURSDAY = new Date("2026-11-26T15:30:00Z");
let now = THURSDAY;
const clock = () => now;

beforeEach(() => {
  now = THURSDAY;
});

function setup(props: Partial<ComposerProps> = {}, scheduling: Partial<ComposerScheduling> = {}) {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  const onTyping = vi.fn();
  const onSchedule = vi.fn<ComposerScheduling["onSchedule"]>(async () => {});
  const onShowScheduled = vi.fn();
  render(
    <Composer
      channelName="general"
      chips={[]}
      onSubmit={onSubmit}
      onRemoveChip={vi.fn()}
      onRetryChip={vi.fn()}
      onReorderChips={vi.fn()}
      onTyping={onTyping}
      scheduling={{
        onSchedule,
        waiting: { count: 0, next: null },
        place: "channel",
        onShowScheduled,
        zone: "Europe/Warsaw",
        clock,
        ...scheduling,
      }}
      {...props}
    />,
  );
  const field = screen.getByRole("combobox", { name: "Napisz na #general" }) as HTMLTextAreaElement;
  return { user, field, onSubmit, onTyping, onSchedule, onShowScheduled };
}

const nextFrame = () => act(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
const sendLater = () => screen.getByRole("menu", { name: "Wyślij później" });

describe("Composer sending later", () => {
  it("schedules the message for a time from the send menu instead of sending it", async () => {
    const { user, field, onSubmit, onTyping, onSchedule } = setup();
    await user.type(field, "Przegląd PR-ów");

    fireEvent.contextMenu(screen.getByRole("button", { name: "Wyślij" }));
    // The presets, each with its short date as drawn, then the calendar.
    expect(within(sendLater()).getAllByRole("menuitem").map((item) => item.textContent)).toEqual([
      "Dziś, 18:00",
      "Jutro, 9:00 pt",
      "Poniedziałek, 9:00 30 lis",
      "Wybierz termin…",
    ]);
    await user.click(within(sendLater()).getByRole("menuitem", { name: /Jutro, 9:00/ }));
    await nextFrame();

    // The time sits in the field, and the button says what it will do.
    expect(screen.getByText("Jutro, 9:00")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Usuń termin" })).toBeInTheDocument();
    onTyping.mockClear();
    await user.type(field, "!");
    // Nobody is told about a message that appears only tomorrow.
    expect(onTyping).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Zaplanuj" }));

    expect(onSchedule).toHaveBeenCalledExactlyOnceWith("Przegląd PR-ów!", new Date("2026-11-27T08:00:00Z"));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(field).toHaveValue("");
    expect(screen.queryByText("Jutro, 9:00")).toBeNull();
  });

  it("opens the menu on a long press, and the press does not also send", async () => {
    const { user, field, onSubmit } = setup();
    await user.type(field, "hej");
    const send = screen.getByRole("button", { name: "Wyślij" });

    fireEvent.pointerDown(send, { pointerType: "touch" });
    await act(() => new Promise((resolve) => setTimeout(resolve, 550)));
    fireEvent.pointerUp(send, { pointerType: "touch" });
    fireEvent.click(send);

    expect(sendLater()).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("takes the time typed after /zaplanuj, and the message is written after it", async () => {
    const { user, field, onSchedule } = setup();

    await user.type(field, "/zaplanuj ");
    await user.type(screen.getByRole("textbox", { name: "kiedy" }), "jutro 9:00{Enter}");
    await nextFrame();
    expect(screen.getByText("Jutro, 9:00")).toBeInTheDocument();

    await user.type(field, "plan{Enter}");
    expect(onSchedule).toHaveBeenCalledExactlyOnceWith("plan", new Date("2026-11-27T08:00:00Z"));
  });

  it("says why a typed time cannot be read, and Enter waits", async () => {
    const { user, field } = setup();

    await user.type(field, "/zaplanuj ");
    const when = screen.getByRole("textbox", { name: "kiedy" });
    await user.type(when, "kiedyś{Enter}");

    expect(when).toBeInTheDocument();
    expect(when).toHaveAccessibleDescription(/Nie rozumiem tego terminu/);
    expect(screen.getByRole("button", { name: "Dalej" })).toBeDisabled();
  });

  it("opens the calendar for /zaplanuj with no time, which offers only the next thirty days", async () => {
    const { user, field, onSchedule } = setup();

    await user.type(field, "/zaplanuj{Enter}");
    await user.keyboard("{Enter}");
    const picker = screen.getByRole("dialog", { name: "Listopad 2026" });
    expect(within(picker).getByRole("button", { name: "środa, 25 listopada 2026" })).toBeDisabled();
    await user.click(within(picker).getByRole("button", { name: "poniedziałek, 30 listopada 2026" }));
    expect(picker).toHaveTextContent("Wyjdzie w poniedziałek 30 listopada o 9:00. Najpóźniej za 30 dni.");

    await user.click(within(picker).getByRole("button", { name: "Następny miesiąc" }));
    expect(within(picker).getByRole("button", { name: "sobota, 26 grudnia 2026" })).toBeEnabled();
    expect(within(picker).getByRole("button", { name: "niedziela, 27 grudnia 2026" })).toBeDisabled();
    await user.click(within(picker).getByRole("button", { name: "sobota, 26 grudnia 2026" }));
    await user.click(within(picker).getByRole("button", { name: "Zaplanuj" }));
    await nextFrame();

    expect(screen.queryByRole("dialog")).toBeNull();
    await user.type(field, "na święta{Enter}");
    expect(onSchedule).toHaveBeenCalledExactlyOnceWith("na święta", new Date("2026-12-26T08:00:00Z"));
  });

  it("keeps the message and the time, and says why, when scheduling is refused", async () => {
    const { user, field } = setup({}, { onSchedule: vi.fn(async () => Promise.reject(new ScheduleRefusedError("limit"))) });
    await user.type(field, "hej");
    fireEvent.contextMenu(screen.getByRole("button", { name: "Wyślij" }));
    await user.click(within(sendLater()).getByRole("menuitem", { name: /Jutro/ }));
    await nextFrame();

    await user.click(screen.getByRole("button", { name: "Zaplanuj" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Masz już tyle zaplanowanych wiadomości, ile można.");
    expect(field).toHaveValue("hej");
    expect(screen.getByText("Jutro, 9:00")).toBeInTheDocument();
  });

  it("does not use a time that passed while the message was written", async () => {
    const { user, field, onSchedule } = setup();
    await user.type(field, "hej");
    fireEvent.contextMenu(screen.getByRole("button", { name: "Wyślij" }));
    await user.click(within(sendLater()).getByRole("menuitem", { name: "Dziś, 18:00" }));
    await nextFrame();

    now = new Date("2026-11-26T17:05:00Z");
    await user.click(screen.getByRole("button", { name: "Zaplanuj" }));

    expect(screen.getByRole("alert")).toHaveTextContent("Ten termin już minął. Wybierz nowy.");
    expect(onSchedule).not.toHaveBeenCalled();
  });

  it("schedules a code message too, fenced as a send would be", async () => {
    const { user, onSchedule } = setup();
    await user.click(screen.getByRole("button", { name: "Wyślij jako kod" }));
    const code = screen.getByRole("textbox", { name: "Napisz na #general" });
    await user.type(code, "/ komentarz");

    fireEvent.contextMenu(screen.getByRole("button", { name: "Wyślij" }));
    await user.click(within(sendLater()).getByRole("menuitem", { name: /Jutro/ }));
    await nextFrame();
    await user.click(screen.getByRole("button", { name: "Zaplanuj" }));

    expect(onSchedule).toHaveBeenCalledExactlyOnceWith(
      expect.stringMatching(/^```\w+\n\/ komentarz\n```$/),
      new Date("2026-11-27T08:00:00Z"),
    );
  });

  it("shows the author what waits here, and opens the list from there", async () => {
    const { user, onShowScheduled } = setup(
      {},
      { waiting: { count: 2, next: new Date("2026-11-27T08:00:00Z") } },
    );

    expect(screen.getByText("2 zaplanowane")).toBeInTheDocument();
    expect(screen.getByText(/na tym kanale, jutro o 9:00/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Pokaż" }));
    expect(onShowScheduled).toHaveBeenCalledOnce();
  });

  it("puts Zaplanuj between the file and the GIF on the phone's sheet", async () => {
    const { user } = setup({ onAttach: vi.fn(), onGif: vi.fn().mockResolvedValue(undefined) });
    // What the phone's stylesheet does: formatting moves into the sheet.
    screen.getByRole("button", { name: "Formatowanie" }).style.display = "none";

    await user.click(screen.getByRole("button", { name: "Komendy i dodawanie" }));
    const sheet = screen.getByRole("dialog", { name: "Komendy i dodawanie" });
    const [file, schedule, gif] = ["Plik", "Zaplanuj", "GIF"].map((name) => within(sheet).getByRole("button", { name }));
    expect(file!.compareDocumentPosition(schedule!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(schedule!.compareDocumentPosition(gif!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("Composer without sending later", () => {
  it("offers neither the menu nor /zaplanuj", async () => {
    const user = userEvent.setup();
    render(
      <Composer
        channelName="general"
        chips={[]}
        onSubmit={vi.fn()}
        onRemoveChip={vi.fn()}
        onRetryChip={vi.fn()}
        onReorderChips={vi.fn()}
      />,
    );
    const field = screen.getByRole("combobox", { name: "Napisz na #general" });
    await user.type(field, "/");
    expect(screen.queryByRole("option", { name: /^\/zaplanuj/ })).toBeNull();

    await user.clear(field);
    await user.type(field, "hej");
    fireEvent.contextMenu(screen.getByRole("button", { name: "Wyślij" }));
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
