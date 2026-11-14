import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey } from "@/i18n";

import ResetPasswordPage from "./page";

const mocks = vi.hoisted(() => ({
  ports: { session: { resetPassword: vi.fn() } },
  query: new URLSearchParams(),
}));

vi.mock("next/navigation", () => ({ useSearchParams: () => mocks.query }));
vi.mock("@/state/hooks", () => ({
  usePorts: () => mocks.ports,
  useLanguage: () => ({ t: (key: MessageKey) => translateMessage("pl", key) }),
}));

function renderWithQuery(query: string) {
  mocks.query = new URLSearchParams(query);
  render(<ResetPasswordPage />);
}

beforeEach(() => {
  vi.resetAllMocks();
});

describe("ResetPasswordPage", () => {
  it("sets the new password with the token from the mailed link", async () => {
    const user = userEvent.setup();
    mocks.ports.session.resetPassword.mockResolvedValue(undefined);
    renderWithQuery("?token=mailed-token");

    await user.type(screen.getByLabelText("Nowe hasło"), "haslo1234");
    await user.type(screen.getByLabelText("Powtórz nowe hasło"), "haslo1234");
    await user.click(screen.getByRole("button", { name: "Ustaw nowe hasło" }));

    expect(mocks.ports.session.resetPassword).toHaveBeenCalledExactlyOnceWith(
      "mailed-token",
      "haslo1234",
    );
  });

  it.each([
    ["is missing", ""],
    ["is empty", "?token="],
    ["is given twice", "?token=one&token=two"],
  ])("treats a token that %s as no token", (_case, query) => {
    renderWithQuery(query);

    expect(screen.getByRole("heading", { name: "Link jest niepełny" })).toBeInTheDocument();
    expect(screen.queryByLabelText("Nowe hasło")).not.toBeInTheDocument();
  });
});
