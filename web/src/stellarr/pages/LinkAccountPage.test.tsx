import { Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey, type MessageValues } from "@/i18n";
import { routes } from "@/routes";

import LinkAccountPage from "./LinkAccountPage";

vi.mock("@/state/hooks", () => ({
  useLanguage: () => ({
    t: (key: MessageKey, values?: MessageValues) => translateMessage("pl", key, values),
  }),
}));

const t = (key: MessageKey) => translateMessage("pl", key);

function show(query: string): MemoryHistory {
  const history = createMemoryHistory(`/link-account${query}`);
  render(
    <Router routes={routes} history={history}>
      <LinkAccountPage />
    </Router>,
  );
  return history;
}

describe("LinkAccountPage", () => {
  it("says the account was linked and leads back to the app", async () => {
    const history = show("?result=linked");

    expect(screen.getByRole("heading", { name: t("linkAccount.linkedTitle") })).toBeVisible();
    expect(screen.getByText(t("linkAccount.linked"))).toBeVisible();
    const back = screen.getByRole("link", { name: t("linkAccount.back") });
    expect(back).toHaveAttribute("href", "/");

    await userEvent.click(back);
    expect(history.navigations).toEqual([{ kind: "push", href: "/" }]);
  });

  it("explains an identity that belongs to another account", () => {
    show("?result=taken");

    expect(screen.getByRole("heading", { name: t("linkAccount.failedTitle") })).toBeVisible();
    expect(screen.getByText(t("linkAccount.taken"))).toBeVisible();
  });

  it.each([
    ["a repeated result", "?result=linked&result=linked"],
    ["an unknown result", "?result=hacked"],
    ["no result", ""],
  ])("reads %s as a failure, never as a success", (_case, query) => {
    show(query);

    expect(screen.getByText(t("linkAccount.failed"))).toBeVisible();
  });
});
