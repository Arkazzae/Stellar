import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey, type MessageValues } from "@/i18n";

import ConfirmEmailPage from "./page";

const mocks = vi.hoisted(() => ({
  confirm: vi.fn<(token: string) => Promise<"changed" | "invalid" | "taken">>(),
  query: new URLSearchParams(),
}));

vi.mock("next/navigation", () => ({ useSearchParams: () => mocks.query }));
vi.mock("@/state/hooks", () => ({
  usePorts: () => ({ session: { confirmEmailChange: mocks.confirm } }),
  useLanguage: () => ({
    t: (key: MessageKey, values?: MessageValues) => translateMessage("pl", key, values),
  }),
}));

const t = (key: MessageKey) => translateMessage("pl", key);

function open(query: string) {
  mocks.query = new URLSearchParams(query);
  render(<ConfirmEmailPage />);
}

beforeEach(() => {
  mocks.confirm = vi.fn(async () => "changed" as const);
});

describe("ConfirmEmailPage", () => {
  it("changes nothing until the button is pressed, then says the address changed", async () => {
    const user = userEvent.setup();
    open("?token=abc");

    expect(mocks.confirm).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: t("confirmEmail.confirm") }));

    expect(mocks.confirm).toHaveBeenCalledExactlyOnceWith("abc");
    expect(await screen.findByRole("heading", { name: t("confirmEmail.changedTitle") })).toBeVisible();
  });

  it("explains a spent or expired link", async () => {
    mocks.confirm = vi.fn(async () => "invalid" as const);
    const user = userEvent.setup();
    open("?token=abc");

    await user.click(screen.getByRole("button", { name: t("confirmEmail.confirm") }));

    expect(await screen.findByText(t("confirmEmail.invalid"))).toBeVisible();
  });

  it("reads a missing or repeated token as an invalid link without asking the server", () => {
    open("?token=a&token=b");
    expect(screen.getByText(t("confirmEmail.invalid"))).toBeVisible();
    cleanup();

    open("");
    expect(screen.getByText(t("confirmEmail.invalid"))).toBeVisible();
    expect(mocks.confirm).not.toHaveBeenCalled();
  });
});
