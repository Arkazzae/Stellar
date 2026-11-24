import { Outlet, Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { PENDING_INVITE_STORAGE_KEY } from "@/lib/auth/invites";
import { ApiError } from "@/lib/api/errors";
import type { InvitePreview } from "@/ports";
import { routes } from "@/routes";
import type { SessionPhase } from "@/state/hooks";
import { polishAsRead } from "@/test/translators";

const mocks = vi.hoisted(() => ({
  session: { phase: "ready" as SessionPhase, reload: vi.fn() },
  ports: {
    directory: { joinInvite: vi.fn(), selectServer: vi.fn() },
    session: { invitePreview: vi.fn(), signInProviders: vi.fn() },
  },
}));

// Where the page sends the visitor; this test is about the invite page alone.
vi.mock("./HomePage", () => ({ default: () => <p>Aplikacja</p> }));
vi.mock("./RegisterPage", () => ({ default: () => <p>Rejestracja</p> }));
vi.mock("./LoginPage", () => ({ default: () => <p>Logowanie</p> }));
vi.mock("@/state/SessionProvider", () => ({
  useSessionContext: () => mocks.session,
}));
vi.mock("@/state/PortsProvider", () => ({
  usePorts: () => mocks.ports,
}));
vi.mock("@/state/LanguageProvider", () => ({
  useLanguage: () => ({ t, locale: "pl" }),
}));

const t = polishAsRead;

const PREVIEW: InvitePreview = {
  server: { id: "s-1", name: "Stellarr Dev", description: "Ekipa od homelabu.", iconUrl: null },
  channel: { id: "c-1", name: "ogólny" },
  inviter: { displayName: "Ada", avatarColor: "oklch(0.58 0.14 24)", avatarUrl: null },
  expiresAt: "2026-11-24T18:00:00Z",
};

let history: MemoryHistory;

/** The app at an invite address: the page leaves when the address does. */
function invite(path: string) {
  history = createMemoryHistory(path);
  return (
    <Router routes={routes} history={history}>
      <Outlet />
    </Router>
  );
}

async function renderInvite(path = "/invite/abc123") {
  await act(async () => {
    render(invite(path));
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.session.phase = "ready";
  window.sessionStorage.clear();
  // A new session port is a new page load: the providers are asked again.
  mocks.ports.session = { invitePreview: vi.fn(), signInProviders: vi.fn() };
  mocks.ports.session.invitePreview.mockResolvedValue(PREVIEW);
  mocks.ports.session.signInProviders.mockResolvedValue([]);
});

describe("InvitePage", () => {
  it("shows a signed-out visitor where the invite leads, and remembers it on the way to an account", async () => {
    const user = userEvent.setup();
    mocks.session.phase = "anonymous";

    await renderInvite("/invite/abc123");

    expect(mocks.ports.session.invitePreview).toHaveBeenCalledExactlyOnceWith("abc123");
    expect(screen.getByRole("heading", { name: "Stellarr Dev" })).toBeVisible();
    expect(screen.getByText(t("auth.invite.from", { inviter: "Ada" }))).toBeVisible();
    // Signed out, the server sends no pictures: the server and the inviter
    // show the marks the app would, and nothing asks for an /assets address.
    expect(screen.getByText("SD")).toBeInTheDocument();
    expect(screen.getByText("AD")).toBeInTheDocument();
    expect(document.querySelectorAll("img:not([src^='/mascot/'])")).toHaveLength(0);
    expect(window.sessionStorage.getItem(PENDING_INVITE_STORAGE_KEY)).toBeNull();
    await user.click(screen.getByRole("button", { name: t("auth.invite.register") }));

    expect(window.sessionStorage.getItem(PENDING_INVITE_STORAGE_KEY)).toBe("abc123");
    expect(history.navigations).toEqual([{ kind: "push", href: "/register" }]);
    expect(mocks.ports.directory.joinInvite).not.toHaveBeenCalled();
  });

  it("brings a visitor who joins through a provider back to the invite", async () => {
    mocks.session.phase = "anonymous";
    mocks.ports.session.signInProviders.mockResolvedValue([
      { displayName: "Authentik", loginUrl: "/api/auth/oidc/login" },
    ]);

    await renderInvite("/invite/abc123");

    expect(await screen.findByRole("link", { name: t("auth.invite.provider", { provider: "Authentik" }) })).toHaveAttribute(
      "href",
      "/api/auth/oidc/login?returnUrl=%2Finvite%2Fabc123",
    );
  });

  it("explains to a signed-out visitor an invite the server does not know, without a way in", async () => {
    mocks.session.phase = "anonymous";
    mocks.ports.session.invitePreview.mockRejectedValue(new ApiError("not-found", "404", 404));

    await renderInvite("/invite/gone");

    expect(screen.getByRole("heading", { name: t("auth.invite.unusableTitle") })).toBeVisible();
    expect(screen.queryByRole("button", { name: t("auth.invite.register") })).toBeNull();
    expect(screen.getByRole("link", { name: t("auth.invite.backToLogin") })).toHaveAttribute("href", "/login");
  });

  it("waits while the session is still loading", async () => {
    mocks.session.phase = "loading";

    await renderInvite();

    expect(screen.getByRole("region", { name: "Wczytywanie serwera…" })).toBeInTheDocument();
    expect(history.navigations).toEqual([]);
    expect(mocks.ports.directory.joinInvite).not.toHaveBeenCalled();
  });

  it("joins the server, selects it, forgets the invite and returns to the app", async () => {
    mocks.ports.directory.joinInvite.mockResolvedValue({ id: "server-7" });
    // Back from an identity provider, the tab still remembers the invite.
    window.sessionStorage.setItem(PENDING_INVITE_STORAGE_KEY, "abc123");

    await renderInvite("/invite/abc123");

    expect(mocks.ports.directory.joinInvite).toHaveBeenCalledExactlyOnceWith("abc123");
    expect(mocks.ports.directory.selectServer).toHaveBeenCalledExactlyOnceWith("server-7");
    expect(window.sessionStorage.getItem(PENDING_INVITE_STORAGE_KEY)).toBeNull();
    expect(history.navigations).toEqual([{ kind: "replace", href: "/" }]);
    expect(mocks.session.reload).toHaveBeenCalledOnce();
    expect(screen.getByText("Aplikacja")).toBeInTheDocument();
  });

  it("joins with the code in the address, decoded as the way back from sign-in encodes it", async () => {
    mocks.ports.directory.joinInvite.mockResolvedValue({ id: "server-7" });

    await renderInvite(`/invite/${encodeURIComponent("old code")}`);

    expect(mocks.ports.directory.joinInvite).toHaveBeenCalledExactlyOnceWith("old code");
  });

  it("explains an invite that does not work and lets the visitor leave", async () => {
    const user = userEvent.setup();
    mocks.ports.directory.joinInvite.mockRejectedValue(new Error("expired"));

    await renderInvite();

    expect(await screen.findByRole("heading", { name: t("auth.invite.unusableTitle") })).toBeVisible();
    expect(mocks.ports.directory.selectServer).not.toHaveBeenCalled();

    await user.click(screen.getByRole("link", { name: t("auth.invite.backToApp") }));
    expect(history.navigations).toEqual([{ kind: "push", href: "/" }]);
  });

  it("explains an address whose code cannot be read, without asking the server", async () => {
    await renderInvite("/invite/%E0%A4%A");

    expect(screen.getByRole("heading", { name: t("auth.invite.unusableTitle") })).toBeVisible();
    expect(mocks.ports.directory.joinInvite).not.toHaveBeenCalled();
  });

  it("does not act on a join that finishes after the page is gone", async () => {
    let finishJoin: (server: { id: string }) => void = () => undefined;
    mocks.ports.directory.joinInvite.mockReturnValue(
      new Promise((resolve) => {
        finishJoin = resolve;
      }),
    );
    await act(async () => {
      render(invite("/invite/abc123")).unmount();
    });

    await act(async () => finishJoin({ id: "server-7" }));

    expect(mocks.ports.directory.selectServer).not.toHaveBeenCalled();
    expect(history.navigations).toEqual([]);
  });
});
