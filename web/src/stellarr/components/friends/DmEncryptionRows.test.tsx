import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { DmEncryptionRows, type DmEncryptionRowsProps } from "./DmEncryptionRows";

function setup(overrides: Partial<DmEncryptionRowsProps> = {}) {
  const props: DmEncryptionRowsProps = {
    peerName: "Maja Nowak",
    state: "off",
    unavailable: null,
    busy: false,
    error: null,
    onSwitch: vi.fn(),
    onOpenCode: vi.fn(),
    ...overrides,
  };
  render(<DmEncryptionRows {...props} />);
  return { props, user: userEvent.setup(), toggle: screen.getByRole("switch", { name: "Szyfrowanie end-to-end" }) };
}

describe("DmEncryptionRows", () => {
  it.each([
    ["off", false, "Włączy się, gdy potwierdzicie je oboje."],
    ["waiting", true, "Czeka, aż Maja Nowak potwierdzi."],
    ["asked", false, "Maja Nowak prosi o szyfrowanie."],
    ["on", true, "Włączone 18 listopada 2026 przez Was oboje."],
  ] as const)("shows %s as the switch %s and says where it stands", (state, checked, text) => {
    const { toggle } = setup({ state, enabledOn: "18 listopada 2026" });
    expect(toggle).toHaveAttribute("aria-checked", String(checked));
    expect(screen.getByText(text, { exact: false })).toBeTruthy();
  });

  it("answers the other person's request by turning on", async () => {
    const asked = setup({ state: "asked" });
    await asked.user.click(asked.toggle);
    expect(asked.props.onSwitch).toHaveBeenCalledWith(true);
  });

  it("offers the safety code only while encryption is on, and opens it", async () => {
    const { user, props } = setup({ state: "on", code: { status: "changed" } });
    await user.click(screen.getByRole("button", { name: /Kod bezpieczeństwa/ }));
    expect(props.onOpenCode).toHaveBeenCalledOnce();
    expect(screen.getByText("Kod się zmienił. Porównajcie go jeszcze raz.")).toBeTruthy();
  });

  it("cannot be switched on where the browser cannot encrypt, and says why", () => {
    const { toggle } = setup({ unavailable: "Ta strona nie jest otwarta przez HTTPS." });
    expect(toggle).toBeDisabled();
    expect(screen.getByText("Ta strona nie jest otwarta przez HTTPS.")).toBeTruthy();
  });
});
