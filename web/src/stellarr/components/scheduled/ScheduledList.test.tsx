import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ScheduleRefusedError, type ScheduledMessage } from "@/ports";

import { ScheduledList, type ScheduledListProps } from "./ScheduledList";

/** Thursday 26 November 2026, 16:30 in Warsaw. */
const NOW = new Date("2026-11-26T15:30:00Z");

const message = (id: string, sendAt: string, extra: Partial<ScheduledMessage> = {}): ScheduledMessage => ({
  id,
  target: { kind: "channel", channelId: "general" },
  content: `Wiadomość ${id}`,
  attachments: [],
  sendAt,
  createdAt: "2026-11-25T10:00:00Z",
  state: "pending",
  ...extra,
});

const ITEMS: ScheduledMessage[] = [
  message("pr", "2026-11-27T08:00:00Z", { content: "Przegląd PR-ów o 10" }),
  message("plan", "2026-11-30T08:00:00Z", {
    target: { kind: "channel", channelId: "announcements" },
    attachments: [{ id: "f", url: "/f", name: "plan.pdf", contentType: "application/pdf", size: 1 } as ScheduledMessage["attachments"][number]],
  }),
  message("duty", "2026-11-24T07:00:00Z", {
    content: "Kto dziś dyżuruje?",
    state: "failed",
    failure: { code: "no_access", at: "2026-11-24T07:00:01Z" },
  }),
];

function setup(overrides: Partial<ScheduledListProps> = {}) {
  const user = userEvent.setup();
  const actions = {
    onSendNow: vi.fn(async () => {}),
    onEdit: vi.fn(async () => {}),
    onReschedule: vi.fn(async () => {}),
    onDelete: vi.fn(async () => {}),
    onCopy: vi.fn(async () => {}),
  };
  render(
    <ScheduledList
      items={ITEMS}
      now={NOW}
      zone="Europe/Warsaw"
      targetOf={(item) => ({
        text: item.target.kind === "channel" ? `# ${item.target.channelId}` : "wiadomość prywatna",
      })}
      {...actions}
      {...overrides}
    />,
  );
  const row = (text: string) => screen.getByText(text).closest("li") as HTMLElement;
  return { user, actions, row };
}

describe("the scheduled list", () => {
  it("counts what waits and what failed, by day, and says why each failure did not go out", () => {
    setup();

    expect(screen.getByText("2 wiadomości czekają, 1 nie wyszła")).toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 3 }).map((heading) => heading.textContent)).toEqual([
      "Jutro, piątek 27 listopada",
      "Poniedziałek, 30 listopada",
      "Nie wyszły",
    ]);
    expect(screen.getByText("# announcements · 1 załącznik")).toBeInTheDocument();
    expect(screen.getByText("# general · nie masz już dostępu do tego kanału")).toBeInTheDocument();
    expect(screen.getByText("wt 8:00")).toBeInTheDocument();
  });

  it("sends now, edits the words and moves the time of a waiting message", async () => {
    const { user, actions, row } = setup();
    const waiting = within(row("Przegląd PR-ów o 10"));

    await user.click(waiting.getByRole("button", { name: "Wyślij teraz" }));
    expect(actions.onSendNow).toHaveBeenCalledWith("pr");

    await user.click(waiting.getByRole("button", { name: "Edytuj" }));
    const words = screen.getByRole("textbox", { name: "Treść zaplanowanej wiadomości" });
    await user.clear(words);
    await user.type(words, "Przegląd o 11");
    await user.click(screen.getByRole("button", { name: "Zapisz" }));
    expect(actions.onEdit).toHaveBeenCalledWith("pr", "Przegląd o 11");

    await user.click(within(row("Wiadomość plan")).getByRole("button", { name: "Zmień termin" }));
    const picker = screen.getByRole("dialog", { name: "Listopad 2026" });
    await user.click(within(picker).getByRole("button", { name: "sobota, 28 listopada 2026" }));
    await user.click(within(picker).getByRole("button", { name: "Zaplanuj" }));
    expect(actions.onReschedule).toHaveBeenCalledWith("plan", new Date("2026-11-28T08:00:00Z"));
  });

  it("offers a failed message only to be copied or deleted", async () => {
    const { user, actions, row } = setup();
    const failed = within(row("Kto dziś dyżuruje?"));

    expect(failed.queryByRole("button", { name: "Edytuj" })).toBeNull();
    expect(failed.queryByRole("button", { name: "Zmień termin" })).toBeNull();
    // Without access to the channel, a second try would fail the same way.
    expect(failed.queryByRole("button", { name: "Wyślij teraz" })).toBeNull();
    await user.click(failed.getByRole("button", { name: "Skopiuj treść" }));
    expect(actions.onCopy).toHaveBeenCalledWith("Kto dziś dyżuruje?");
    expect(failed.getByRole("button", { name: "Skopiowano" })).toBeInTheDocument();

    await user.click(failed.getByRole("button", { name: "Usuń" }));
    expect(actions.onDelete).toHaveBeenCalledWith("duty");
  });

  it("sends a failed message again when nothing lasting stopped it", async () => {
    const { user, actions, row } = setup({
      items: [
        message("retry", "2026-11-24T07:00:00Z", {
          content: "Spróbuj jeszcze raz",
          state: "failed",
          failure: { code: "other", at: "2026-11-24T07:00:01Z" },
        }),
      ],
    });

    await user.click(within(row("Spróbuj jeszcze raz")).getByRole("button", { name: "Wyślij teraz" }));

    expect(actions.onSendNow).toHaveBeenCalledWith("retry");
  });

  it("says why an action was refused", async () => {
    const { user, row } = setup({
      onSendNow: vi.fn(async () => Promise.reject(new ScheduleRefusedError("slowMode"))),
    });

    await user.click(within(row("Przegląd PR-ów o 10")).getByRole("button", { name: "Wyślij teraz" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Na tym kanale działa tryb powolny.");
  });
});
