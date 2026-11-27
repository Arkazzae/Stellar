import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, type Mock } from "vitest";

import { EventRefusedError, type EventDraft } from "@/ports";
import { polish } from "@/test/translators";

import { EventSheet } from "./EventSheet";

/** Thursday 26 November 2026, 13:30 in Warsaw. */
const NOW = new Date("2026-11-26T12:30:00Z");

function setup({
  initial,
  onSave = vi.fn(async (_draft: EventDraft) => {}),
}: { initial?: EventDraft; onSave?: Mock<(draft: EventDraft) => Promise<void>> } = {}) {
  const user = userEvent.setup();
  const onClose = vi.fn();
  render(
    <EventSheet
      serverName="Stellarr Dev"
      voiceChannels={[
        { id: "vc-gaming", name: "Gaming" },
        { id: "vc-meet", name: "Spotkania" },
      ]}
      textChannels={[
        { id: "tc-general", name: "general" },
        { id: "tc-news", name: "announcements" },
      ]}
      now={NOW}
      zone="Europe/Warsaw"
      announceOn="tc-general"
      {...(initial === undefined ? {} : { initial })}
      onSave={onSave}
      onClose={onClose}
      locale="pl"
      t={polish}
    />,
  );
  const sheet = screen.getByRole("dialog", { name: initial === undefined ? "Nowe wydarzenie" : "Zmień wydarzenie" });
  return { user, sheet, onSave, onClose };
}

describe("the new event sheet", () => {
  it("creates an event on a voice channel tomorrow evening, announced where it was asked for", async () => {
    const { user, sheet, onSave, onClose } = setup();
    expect(within(sheet).getByText("Na serwerze Stellarr Dev")).toBeInTheDocument();
    const create = within(sheet).getByRole("button", { name: "Utwórz wydarzenie" });
    expect(create).toBeDisabled();

    await user.type(within(sheet).getByRole("textbox", { name: "Nazwa" }), "Wieczór planszówek");
    await user.click(create);

    expect(onSave).toHaveBeenCalledExactlyOnceWith({
      name: "Wieczór planszówek",
      place: { kind: "voice", channelId: "vc-gaming" },
      // Friday 27 November, 20:00 in Warsaw.
      startsAt: new Date("2026-11-27T19:00:00Z"),
      timeZone: "Europe/Warsaw",
      repeat: "none",
      announceChannelId: "tc-general",
    });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("takes a place somewhere else only with an end, which may fall on the next day", async () => {
    const { user, sheet, onSave } = setup();
    await user.type(within(sheet).getByRole("textbox", { name: "Nazwa" }), "Nocne granie");
    await user.click(within(sheet).getByRole("button", { name: /Gdzie indziej/ }));
    await user.type(within(sheet).getByRole("textbox", { name: "Adres albo link" }), "Kawiarnia Kafka");
    const create = within(sheet).getByRole("button", { name: "Utwórz wydarzenie" });
    expect(within(sheet).getByText("Podaj, o której się kończy.")).toBeInTheDocument();
    expect(create).toBeDisabled();

    fireEvent.change(within(sheet).getByLabelText("Koniec"), { target: { value: "01:30" } });
    expect(within(sheet).getByText("Kończy się następnego dnia.")).toBeInTheDocument();
    await user.click(create);

    expect(onSave.mock.calls[0]?.[0]).toMatchObject({
      place: { kind: "elsewhere", text: "Kawiarnia Kafka" },
      startsAt: new Date("2026-11-27T19:00:00Z"),
      endsAt: new Date("2026-11-28T00:30:00Z"),
    });
  });

  it("limits the seats, repeats, and announces nowhere when asked", async () => {
    const { user, sheet, onSave } = setup();
    await user.type(within(sheet).getByRole("textbox", { name: "Nazwa" }), "Przegląd sprintu");
    await user.click(within(sheet).getByRole("button", { name: "Limit" }));
    const seats = within(sheet).getByRole("spinbutton", { name: "Liczba miejsc" });
    await user.clear(seats);
    await user.type(seats, "12");
    await user.click(within(sheet).getByRole("button", { name: "Co tydzień" }));
    await user.click(within(sheet).getByRole("combobox", { name: /Ogłoś na kanale/ }));
    await user.click(screen.getByRole("option", { name: "Nie ogłaszaj" }));
    await user.click(within(sheet).getByRole("button", { name: "Utwórz wydarzenie" }));

    const draft = onSave.mock.calls[0]?.[0];
    expect(draft).toMatchObject({ seats: 12, repeat: "weekly" });
    expect(draft).not.toHaveProperty("announceChannelId");
  });

  it("picks the day from a month that starts today, and Escape closes only the calendar", async () => {
    const { user, sheet, onSave } = setup();
    await user.type(within(sheet).getByRole("textbox", { name: "Nazwa" }), "Wieczór planszówek");

    await user.click(within(sheet).getByRole("button", { name: /^Dzień:/ }));
    const calendar = screen.getByRole("dialog", { name: "Listopad 2026" });
    expect(within(calendar).getByRole("button", { name: "środa, 25 listopada 2026" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Listopad 2026" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "Nowe wydarzenie" })).toBeInTheDocument();

    await user.click(within(sheet).getByRole("button", { name: /^Dzień:/ }));
    await user.click(screen.getByRole("button", { name: "sobota, 28 listopada 2026" }));
    expect(within(sheet).getByRole("button", { name: /^Dzień:/ })).toHaveTextContent("sob., 28 listopada");
    await user.click(within(sheet).getByRole("button", { name: "Utwórz wydarzenie" }));

    expect(onSave.mock.calls[0]?.[0].startsAt).toEqual(new Date("2026-11-28T19:00:00Z"));
  });

  it("changes an event it was given, and says why the server would not", async () => {
    const onSave = vi.fn(async (_draft: EventDraft) => Promise.reject(new EventRefusedError("forbidden")));
    const { user, sheet } = setup({
      onSave,
      initial: {
        name: "Wieczór planszówek",
        place: { kind: "voice", channelId: "vc-meet" },
        startsAt: new Date("2026-11-28T19:00:00Z"),
        endsAt: new Date("2026-11-28T21:00:00Z"),
        timeZone: "Europe/Warsaw",
        seats: 10,
        repeat: "none",
      },
    });
    expect(within(sheet).getByRole("textbox", { name: "Nazwa" })).toHaveValue("Wieczór planszówek");
    expect(within(sheet).getByRole("spinbutton", { name: "Liczba miejsc" })).toHaveValue(10);

    await user.click(within(sheet).getByRole("button", { name: "Zapisz zmiany" }));

    expect(await within(sheet).findByRole("alert")).toHaveTextContent("Nie możesz tworzyć wydarzeń na tym serwerze.");
  });

  it("saves a running event's new name, keeping the time it already has", async () => {
    const { user, sheet, onSave } = setup({
      initial: {
        name: "Wieczór planszówek",
        place: { kind: "voice", channelId: "vc-gaming" },
        // Begun an hour before the sheet was opened.
        startsAt: new Date("2026-11-26T11:30:00Z"),
        timeZone: "Europe/Warsaw",
        repeat: "none",
      },
    });
    const name = within(sheet).getByRole("textbox", { name: "Nazwa" });
    await user.clear(name);
    await user.type(name, "Wieczór gier");
    await user.click(within(sheet).getByRole("button", { name: "Zapisz zmiany" }));

    expect(onSave.mock.calls[0]?.[0]).toMatchObject({ name: "Wieczór gier", startsAt: new Date("2026-11-26T11:30:00Z") });
  });
});
