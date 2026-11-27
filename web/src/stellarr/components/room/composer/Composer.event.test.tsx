import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Composer, type ComposerProps } from "./Composer";

function setup(props: Partial<ComposerProps> = {}) {
  const user = userEvent.setup();
  const onEvent = vi.fn();
  render(
    <Composer
      channelName="general"
      chips={[]}
      onSubmit={vi.fn()}
      onRemoveChip={vi.fn()}
      onRetryChip={vi.fn()}
      onReorderChips={vi.fn()}
      onEvent={onEvent}
      {...props}
    />,
  );
  const field = screen.getByRole("combobox", { name: "Napisz na #general" });
  return { user, field, onEvent };
}

describe("Composer's event command", () => {
  it("opens a new event from /wydarzenie and from the English name", async () => {
    const { user, field, onEvent } = setup();

    await user.type(field, "/wydarzenie{Enter}");
    expect(onEvent).toHaveBeenCalledOnce();
    expect(field).toHaveValue("");

    await user.type(field, "/event{Enter}");
    expect(onEvent).toHaveBeenCalledTimes(2);
  });

  it("is not offered to whoever may not create events", async () => {
    const { user, field } = setup({ onEvent: undefined });

    await user.type(field, "/wyd");

    expect(screen.queryByRole("option", { name: /^\/wydarzenie/ })).toBeNull();
  });

  it("gives the phone's sheet the board's four tiles, with GIF back among the message's rows", async () => {
    const { user } = setup({
      onAttach: vi.fn(),
      onGif: vi.fn().mockResolvedValue(undefined),
      onPoll: vi.fn(),
      scheduling: {
        onSchedule: vi.fn(),
        waiting: { count: 0, next: null },
        place: "channel",
        onShowScheduled: vi.fn(),
        zone: "Europe/Warsaw",
      },
    });
    // What the phone's stylesheet does: formatting moves into the sheet.
    screen.getByRole("button", { name: "Formatowanie" }).style.display = "none";

    await user.click(screen.getByRole("button", { name: "Komendy i dodawanie" }));
    const sheet = screen.getByRole("dialog", { name: "Komendy i dodawanie" });
    const tiles = ["Plik", "Ankieta", "Wydarzenie", "Zaplanuj"].map((name) => within(sheet).getByRole("button", { name }));
    for (const [before, after] of tiles.slice(0, -1).map((tile, index) => [tile, tiles[index + 1]!] as const)) {
      expect(before.compareDocumentPosition(after) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
    const message = within(sheet).getByRole("group", { name: "Wiadomość" });
    expect(within(message).getByRole("button", { name: /GIF/ })).toBeInTheDocument();
  });
});
