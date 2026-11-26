import { Router, usePathname } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey, type MessageValues } from "@/i18n";
import { routes } from "@/routes";
import type { SessionPhase } from "@/state/hooks";

import HomePage from "./HomePage";

const mocks = vi.hoisted(() => ({
  session: {
    phase: "loading" as SessionPhase,
    error: null as string | null,
    reload: vi.fn(),
    logout: vi.fn(async () => {}),
  },
}));

vi.mock("@/state/SessionProvider", () => ({
  useSessionContext: () => mocks.session,
}));
vi.mock("@/state/LanguageProvider", () => ({
  useLanguage: () => ({
    t: (key: MessageKey, values?: MessageValues) => translateMessage("pl", key, values),
  }),
}));
// The socket and voice code that comes with the app is not what this page checks.
vi.mock("@/adapters/signedInLoader", () => ({ loadSignedInCode: async () => ({}) }));
vi.mock("@/containers/AppContainer", () => ({
  AppContainer: () => <div data-testid="app" />,
}));

let history: MemoryHistory;

function ActiveHome() {
  return usePathname() === "/" ? <HomePage /> : null;
}

function renderHome(path = "/") {
  history = createMemoryHistory(path);
  return render(
    <Router routes={routes} history={history}>
      <ActiveHome />
    </Router>,
  );
}

function settleAs(phase: SessionPhase, error: string | null = null) {
  mocks.session.phase = phase;
  mocks.session.error = error;
}

beforeEach(() => {
  settleAs("loading");
  mocks.session.reload = vi.fn();
  mocks.session.logout = vi.fn(async () => {});
});

describe("HomePage", () => {
  it("opens the app once the session is ready, with the skeleton until its code is there", async () => {
    settleAs("ready");
    renderHome();

    // The app's code arrives on its own, after the first render.
    expect(screen.getByRole("status")).toHaveTextContent("Wczytywanie serwera…");
    expect(await screen.findByTestId("app")).toBeInTheDocument();
    expect(history.navigations).toEqual([]);
  });

  it("explains a failed boot and offers a retry", async () => {
    settleAs("error", "Serwer nie odpowiada.");
    renderHome();

    expect(screen.getByText("Nie udało się wczytać serwera")).toBeInTheDocument();
    expect(screen.getByText("Serwer nie odpowiada.")).toBeInTheDocument();
    expect(screen.queryByTestId("app")).not.toBeInTheDocument();

    await userEvent.setup().click(screen.getByRole("button", { name: "Spróbuj ponownie" }));

    expect(mocks.session.reload).toHaveBeenCalledOnce();
    expect(mocks.session.logout).not.toHaveBeenCalled();
  });

  it("falls back to a generic hint when the failure has no message", () => {
    settleAs("error");
    renderHome();

    expect(
      screen.getByText("Sprawdź połączenie i spróbuj ponownie za chwilę."),
    ).toBeInTheDocument();
  });

  it("tells a revoked account so and offers to log out instead of retrying", async () => {
    settleAs("revoked", "To konto nie ma już dostępu do tego serwera.");
    renderHome();

    expect(screen.getByText("Brak dostępu do serwera")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Spróbuj ponownie" })).not.toBeInTheDocument();

    await userEvent.setup().click(screen.getByRole("button", { name: "Wyloguj się" }));

    expect(mocks.session.logout).toHaveBeenCalledOnce();
    expect(mocks.session.reload).not.toHaveBeenCalled();
  });

  it("sends an anonymous visitor to the sign-in page", () => {
    settleAs("anonymous");
    renderHome();

    expect(history.navigations).toEqual([{ kind: "replace", href: "/login" }]);
    expect(screen.queryByTestId("app")).not.toBeInTheDocument();
  });

  it("preserves a notification through sign-in", () => {
    const target = "/?notice=00000001-2222-3333-4444-000000000005";
    settleAs("anonymous");
    renderHome(target);
    expect(history.navigations).toEqual([{ kind: "replace", href: "/login?returnUrl=" + encodeURIComponent(target) }]);
  });

  it("retries the offline start when the connection returns", () => {
    const online = vi.spyOn(navigator, "onLine", "get").mockReturnValue(false);
    try {
      settleAs("error");
      renderHome();
      expect(screen.getByText("Jesteś offline")).toBeVisible();
      fireEvent(window, new Event("online"));
      expect(mocks.session.reload).toHaveBeenCalledOnce();
    } finally {
      online.mockRestore();
    }
  });

  it("shows the loading skeleton while the session settles, without leaving", () => {
    settleAs("loading");
    renderHome();

    expect(screen.getByRole("status")).toHaveTextContent("Wczytywanie serwera…");
    expect(history.navigations).toEqual([]);
  });
});
