import { Router } from "@stellarr/router";
import { createMemoryHistory } from "@stellarr/router/testing";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { NotFoundPage } from "./not-found-page";
import { selectPolish } from "../test/locale";
import { routes } from "../routes";

selectPolish();

describe("NotFoundPage", () => {
  it("leads back to the overview", async () => {
    const history = createMemoryHistory("/cockpit/nie-ma-takiego-adresu");
    render(
      <Router routes={routes} history={history}>
        <NotFoundPage />
      </Router>,
    );
    expect(screen.getByRole("heading", { name: "Nie ma takiego widoku" })).toBeTruthy();

    const back = screen.getByRole("link", { name: "Wróć do przeglądu" });
    expect(back.getAttribute("href")).toBe("/cockpit");
    await userEvent.click(back);

    expect(history.navigations).toEqual([{ kind: "push", href: "/cockpit" }]);
  });
});
