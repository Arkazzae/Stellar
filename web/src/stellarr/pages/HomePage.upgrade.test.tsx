import { Router } from "@stellarr/router";
import { createMemoryHistory } from "@stellarr/router/testing";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey } from "@/i18n";
import { routes } from "@/routes";

import HomePage from "./HomePage";

vi.mock("@/state/SessionProvider", () => ({
  useSessionContext: () => ({ phase: "ready", error: null, reload: vi.fn(), logout: vi.fn() }),
}));
vi.mock("@/state/LanguageProvider", () => ({
  useLanguage: () => ({ t: (key: MessageKey) => translateMessage("pl", key) }),
}));
// The app's code cannot be had: the network failed, or the server has moved
// on to a release whose files this tab does not name.
vi.mock("@/containers/AppContainer", () => {
  throw new Error("Failed to fetch dynamically imported module");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("HomePage when the app's code cannot be loaded", () => {
  it("says so and loads the page again on request, instead of going blank", async () => {
    const reload = vi.fn();
    vi.stubGlobal("location", { ...window.location, reload });
    render(
      <Router routes={routes} history={createMemoryHistory("/")}>
        <HomePage />
      </Router>,
    );

    expect(await screen.findByText("Nie udało się wczytać serwera")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Spróbuj ponownie" }));

    expect(reload).toHaveBeenCalledOnce();
  });
});
