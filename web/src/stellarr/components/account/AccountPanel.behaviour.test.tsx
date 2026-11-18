import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AccountPanel, type AccountPanelProps } from "./AccountPanel";

function props(overrides: Partial<AccountPanelProps> = {}): AccountPanelProps {
  return {
    presence: "online",
    customStatus: "",
    onSetPresence: vi.fn(),
    onSaveCustomStatus: vi.fn(),
    onLogout: vi.fn(),
    ...overrides,
  };
}

const SERVER_EMOJI = [
  { id: "emoji-party", shortcode: ":party:", glyph: "/emoji/party.webp" },
  { id: "emoji-cat", shortcode: ":cat:", glyph: "/emoji/cat.webp" },
];

const statusField = () => screen.getByRole("textbox", { name: "Twój status" });
const emojiSlot = () => screen.getByRole("button", { name: /^(Dodaj|Wybierz) emoji$/ });

/** A recent pick is listed twice in the catalogue; either one will do. */
async function pickEmoji(user: UserEvent, name: string) {
  await user.click(emojiSlot());
  const [option] = screen.getAllByRole("option", { name });
  if (option === undefined) throw new Error(`no ${name} in the catalogue`);
  await user.click(option);
}

afterEach(() => {
  vi.useRealTimers();
});

describe("AccountPanel status line", () => {
  it("saves the whole line on Enter, emoji included, and keeps it on one line", async () => {
    const user = userEvent.setup();
    const onSaveCustomStatus = vi.fn();
    render(
      <AccountPanel {...props({ customStatus: "🎧 stary", onSaveCustomStatus })} />,
    );

    await user.clear(statusField());
    await user.type(statusField(), "  w pracy");
    await user.paste("\nod rana  ");
    expect(statusField()).toHaveValue("w pracy od rana  ");

    await user.keyboard("{Enter}");

    expect(onSaveCustomStatus).toHaveBeenCalledExactlyOnceWith("🎧 w pracy od rana");
    expect(statusField()).toHaveValue("w pracy od rana  ");
  });

  it("leaves the Enter that ends a composition to the input method", async () => {
    const user = userEvent.setup();
    const onSaveCustomStatus = vi.fn();
    render(<AccountPanel {...props({ onSaveCustomStatus })} />);

    await user.type(statusField(), "にほん");
    fireEvent.keyDown(statusField(), { key: "Enter", isComposing: true });

    expect(onSaveCustomStatus).not.toHaveBeenCalled();
  });

  it("clears the saved status at once", async () => {
    const user = userEvent.setup();
    const onSaveCustomStatus = vi.fn();
    render(
      <AccountPanel {...props({ customStatus: "🎧 w pracy", onSaveCustomStatus })} />,
    );

    await user.click(screen.getByRole("button", { name: "Wyczyść status" }));

    expect(onSaveCustomStatus).toHaveBeenCalledExactlyOnceWith("");
  });

  it("takes only the emoji off with Backspace at the start of the words", async () => {
    const user = userEvent.setup();
    const onSaveCustomStatus = vi.fn();
    render(
      <AccountPanel
        {...props({ customStatus: "🛠️ w warsztacie", onSaveCustomStatus })}
      />,
    );

    await user.type(statusField(), "{Backspace}", {
      initialSelectionStart: 0,
      initialSelectionEnd: 0,
    });

    expect(emojiSlot()).toHaveAccessibleName("Dodaj emoji");
    expect(statusField()).toHaveValue("w warsztacie");
    await user.keyboard("{Enter}");
    expect(onSaveCustomStatus).toHaveBeenCalledExactlyOnceWith("w warsztacie");
  });

  it("sets, replaces and removes the leading emoji from the picker", async () => {
    const user = userEvent.setup();
    const onSaveCustomStatus = vi.fn();
    render(
      <AccountPanel
        {...props({
          customStatus: ":gone: hej",
          customEmoji: SERVER_EMOJI,
          onSaveCustomStatus,
        })}
      />,
    );

    // The server has no `:gone:`, so there is no picture for the slot.
    expect(statusField()).toHaveValue(":gone: hej");
    expect(emojiSlot()).toHaveAccessibleName("Dodaj emoji");

    await pickEmoji(user, ":party:");
    expect(within(emojiSlot()).getByRole("img", { name: ":party:" })).toBeVisible();
    // A mouse pick puts the caret back at the end of the words.
    expect(statusField()).toHaveFocus();
    expect(statusField()).toHaveProperty("selectionStart", ":gone: hej".length);

    await pickEmoji(user, ":cat:");
    expect(within(emojiSlot()).getByRole("img", { name: ":cat:" })).toBeVisible();
    expect(statusField()).toHaveValue(":gone: hej");
    await user.keyboard("{Enter}");
    expect(onSaveCustomStatus).toHaveBeenCalledExactlyOnceWith(":cat: :gone: hej");

    await pickEmoji(user, ":cat:");
    expect(emojiSlot()).toHaveAccessibleName("Dodaj emoji");
    expect(statusField()).toHaveValue(":gone: hej");
  });

  it("confirms a save with a tick that cannot be pressed, then offers to clear", async () => {
    const onSaveCustomStatus = vi.fn();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const { rerender } = render(
      <AccountPanel {...props({ onSaveCustomStatus })} />,
    );

    await user.type(statusField(), "praca");
    await user.click(screen.getByRole("button", { name: "Zapisz" }));
    expect(onSaveCustomStatus).toHaveBeenCalledExactlyOnceWith("praca");

    // The parent stores the status; until the tick goes, nothing is a button.
    rerender(
      <AccountPanel {...props({ onSaveCustomStatus, customStatus: "praca" })} />,
    );
    expect(screen.getByTitle("Zapisano").closest("button")).toBeNull();
    expect(screen.queryByRole("button", { name: "Zapisz" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Wyczyść status" })).toBeNull();

    await act(() => vi.advanceTimersByTimeAsync(1600));
    expect(screen.queryByTitle("Zapisano")).toBeNull();
    expect(screen.getByRole("button", { name: "Wyczyść status" })).toBeVisible();
  });

  it("counts the emoji towards the limit and shows the count near it", async () => {
    const user = userEvent.setup();
    const onSaveCustomStatus = vi.fn();
    render(
      <AccountPanel
        {...props({
          customStatus: "a".repeat(99),
          customEmoji: SERVER_EMOJI,
          onSaveCustomStatus,
        })}
      />,
    );

    expect(screen.queryByText(/\/128$/)).toBeNull();
    expect(statusField()).toHaveAttribute("maxlength", "128");

    await user.type(statusField(), "b");
    expect(screen.getByText("100/128")).toBeVisible();

    // An emoji typed in front of 126 units fits, until it moves to the slot
    // and brings its space: the words give way, typed or picked.
    await user.type(statusField(), "c".repeat(26));
    await user.type(statusField(), "🎧", {
      initialSelectionStart: 0,
      initialSelectionEnd: 0,
    });
    expect(statusField()).toHaveAttribute("maxlength", "125");
    expect(screen.getByText("128/128")).toBeVisible();
    await user.keyboard("{Enter}");
    expect(onSaveCustomStatus).toHaveBeenCalledExactlyOnceWith(
      `🎧 ${"a".repeat(99)}b${"c".repeat(25)}`,
    );

    await pickEmoji(user, ":party:");
    expect(statusField()).toHaveAttribute("maxlength", "120");
    expect(statusField()).toHaveValue(`${"a".repeat(99)}b${"c".repeat(20)}`);
    expect(screen.getByText("128/128")).toBeVisible();
  });

  it("starts the draft over when the saved status changes elsewhere", () => {
    const { rerender } = render(
      <AccountPanel {...props({ customStatus: "pierwszy" })} />,
    );
    fireEvent.change(statusField(), { target: { value: "w trakcie pisania" } });

    rerender(<AccountPanel {...props({ customStatus: "z innej karty" })} />);

    expect(statusField()).toHaveValue("z innej karty");
  });
});
