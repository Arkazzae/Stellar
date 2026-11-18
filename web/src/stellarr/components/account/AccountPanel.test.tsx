import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AccountPanel, type AccountPanelProps } from "./AccountPanel";

function setup(overrides: Partial<AccountPanelProps> = {}) {
  const props: AccountPanelProps = {
    presence: "online",
    customStatus: "",
    onSetPresence: vi.fn(),
    onSaveCustomStatus: vi.fn(),
    onLogout: vi.fn(),
    ...overrides,
  };
  render(<AccountPanel {...props} />);
  return { props, user: userEvent.setup() };
}

const statusField = () => screen.getByRole("textbox", { name: "Twój status" });

describe("AccountPanel", () => {
  it("splits a saved status into its emoji and words, and offers to clear it", async () => {
    const { props, user } = setup({ customStatus: "🧑‍💻 koduję" });

    // A ZWJ sequence is one emoji, not a pictograph and a stray joiner.
    expect(statusField()).toHaveValue("koduję");
    const slot = screen.getByRole("button", { name: "Wybierz emoji" });
    expect(slot).toHaveTextContent("🧑‍💻");
    expect(slot).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("button", { name: "Wyczyść status" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Zapisz" })).toBeNull();

    await user.type(statusField(), "{Enter}");
    expect(props.onSaveCustomStatus).not.toHaveBeenCalled();

    await user.type(statusField(), " i testuję");
    expect(screen.getByRole("button", { name: "Zapisz" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Wyczyść status" })).toBeNull();
  });

  it("marks the chosen availability, reports a pick and signs out", async () => {
    const { props, user } = setup({ presence: "dnd" });

    const availability = within(
      screen.getByRole("group", { name: "Twoja dostępność" }),
    );
    expect(
      availability.getByRole("button", { name: "Nie przeszkadzać" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(availability.getByRole("button", { name: "Online" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );

    await user.click(availability.getByRole("button", { name: "Zaraz wracam" }));
    await user.click(screen.getByRole("button", { name: "Wyloguj się" }));

    expect(props.onSetPresence).toHaveBeenCalledExactlyOnceWith("idle");
    expect(props.onLogout).toHaveBeenCalledOnce();
  });

  it("keeps the software keyboard down when emoji are picked by touch", () => {
    setup();

    const trigger = screen.getByRole("button", { name: "Dodaj emoji" });
    fireEvent.pointerDown(trigger, { pointerType: "touch" });
    fireEvent.click(trigger, { detail: 1 });

    expect(
      screen.getByRole("textbox", { name: "Szukaj emoji" }),
    ).not.toHaveFocus();

    const [option] = screen.getAllByRole("option");
    if (option === undefined) throw new Error("the catalogue is empty");
    fireEvent.click(option);

    expect(screen.getByRole("button", { name: "Wybierz emoji" })).toHaveFocus();
    expect(statusField()).not.toHaveFocus();
  });

  it("focuses emoji search from a keyboard, and gives focus back to the slot on Escape", async () => {
    const { user } = setup();

    const trigger = screen.getByRole("button", { name: "Dodaj emoji" });
    fireEvent.click(trigger, { detail: 0 });

    await vi.waitFor(() => {
      expect(
        screen.getByRole("textbox", { name: "Szukaj emoji" }),
      ).toHaveFocus();
    });

    await user.keyboard("{Escape}");
    expect(trigger).toHaveFocus();
  });
});
