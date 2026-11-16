import { Router } from "@stellarr/router";
import { createMemoryHistory } from "@stellarr/router/testing";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey, type MessageValues } from "@/i18n";
import { routes } from "@/routes";

import ConfirmEmailPage from "./ConfirmEmailPage";

const mocks = vi.hoisted(() => ({ confirm: vi.fn<(token: string) => Promise<"changed" | "invalid" | "taken">>() }));

vi.mock("@/state/hooks", () => ({
  usePorts: () => ({ session: { confirmEmailChange: mocks.confirm } }),
  useLanguage: () => ({
    t: (key: MessageKey, values?: MessageValues) => translateMessage("pl", key, values),
  }),
}));

const t = (key: MessageKey) => translateMessage("pl", key);

function open(query: string) {
  render(
    <Router routes={routes} history={createMemoryHistory(`/confirm-email${query}`)}>
      <ConfirmEmailPage />
    </Router>,
  );
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
    expect(screen.getByRole("link", { name: t("linkAccount.back") })).toHaveAttribute("href", "/");
  });

  it("explains a spent or expired link", async () => {
    mocks.confirm = vi.fn(async () => "invalid" as const);
    const user = userEvent.setup();
    open("?token=abc");

    await user.click(screen.getByRole("button", { name: t("confirmEmail.confirm") }));

    expect(await screen.findByText(t("confirmEmail.invalid"))).toBeVisible();
  });

  it.each([
    ["missing", ""],
    ["empty", "?token="],
    ["repeated", "?token=a&token=b"],
  ])("reads a %s token as an invalid link without asking the server", (_case, query) => {
    open(query);

    expect(screen.getByText(t("confirmEmail.invalid"))).toBeVisible();
    expect(mocks.confirm).not.toHaveBeenCalled();
  });
});
