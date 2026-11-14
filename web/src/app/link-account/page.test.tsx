import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey, type MessageValues } from "@/i18n";

import LinkAccountPage from "./page";

const mocks = vi.hoisted(() => ({ query: new URLSearchParams() }));

vi.mock("next/navigation", () => ({ useSearchParams: () => mocks.query }));
vi.mock("@/state/hooks", () => ({
  useLanguage: () => ({
    t: (key: MessageKey, values?: MessageValues) => translateMessage("pl", key, values),
  }),
}));

const t = (key: MessageKey) => translateMessage("pl", key);

function show(query: string) {
  mocks.query = new URLSearchParams(query);
  render(<LinkAccountPage />);
}

describe("LinkAccountPage", () => {
  it("says the account was linked and leads back to the app", () => {
    show("?result=linked");

    expect(screen.getByRole("heading", { name: t("linkAccount.linkedTitle") })).toBeVisible();
    expect(screen.getByText(t("linkAccount.linked"))).toBeVisible();
    expect(screen.getByRole("link", { name: t("linkAccount.back") })).toHaveAttribute("href", "/");
  });

  it("explains an identity that belongs to another account", () => {
    show("?result=taken");

    expect(screen.getByRole("heading", { name: t("linkAccount.failedTitle") })).toBeVisible();
    expect(screen.getByText(t("linkAccount.taken"))).toBeVisible();
  });

  it.each([
    ["a repeated result", "?result=linked&result=linked"],
    ["an unknown result", "?result=granted"],
    ["no result", ""],
  ])("reads %s as a failure, never as a success", (_case, query) => {
    show(query);
    expect(screen.getByRole("heading", { name: t("linkAccount.failedTitle") })).toBeVisible();
    expect(screen.getByText(t("linkAccount.failed"))).toBeVisible();
  });
});
