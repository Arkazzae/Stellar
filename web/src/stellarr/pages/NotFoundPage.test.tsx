import { Router } from "@stellarr/router";
import { createMemoryHistory } from "@stellarr/router/testing";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { routes } from "@/routes";
import { LanguageProvider } from "@/state/LanguageProvider";

import NotFound from "./NotFoundPage";

describe("the page for an address that leads nowhere", () => {
  it("speaks the language the provider holds", () => {
    const { unmount } = render(
      <LanguageProvider initialLocale="en">
        <NotFound />
      </LanguageProvider>,
    );
    expect(screen.getByRole("heading", { name: /This page does not exist/ })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Back to the conversation" })).toHaveAttribute(
      "href",
      "/",
    );
    unmount();

    render(
      <LanguageProvider initialLocale="pl">
        <NotFound />
      </LanguageProvider>,
    );
    expect(screen.getByRole("heading", { name: /Taka strona nie istnieje/ })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Wróć do rozmowy" })).toBeInTheDocument();
  });

  it("leads back to the conversation without loading the page again", async () => {
    const history = createMemoryHistory("/nie-ma-takiego-adresu");
    render(
      <Router routes={routes} history={history}>
        <NotFound />
      </Router>,
    );

    await userEvent.click(screen.getByRole("link", { name: "Wróć do rozmowy" }));

    expect(history.navigations).toEqual([{ kind: "push", href: "/" }]);
  });
});
