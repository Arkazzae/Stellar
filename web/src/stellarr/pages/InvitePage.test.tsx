import { Outlet, Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { PENDING_INVITE_STORAGE_KEY } from "@/lib/auth/invites";
import { translateMessage, type MessageKey } from "@/i18n";
import { routes } from "@/routes";
import type { SessionPhase } from "@/state/hooks";

const mocks = vi.hoisted(() => ({
  session: { phase: "ready" as SessionPhase, reload: vi.fn() },
  ports: { directory: { joinInvite: vi.fn(), selectServer: vi.fn() } },
}));

// Where the page sends the visitor; this test is about the invite page alone.
vi.mock("./HomePage", () => ({ default: () => <p>Aplikacja</p> }));
vi.mock("./RegisterPage", () => ({ default: () => <p>Rejestracja</p> }));
vi.mock("@/state/hooks", () => ({
  useSession: () => mocks.session,
  usePorts: () => mocks.ports,
  useLanguage: () => ({ t: (key: MessageKey) => translateMessage("pl", key) }),
}));

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
});

describe("InvitePage", () => {
  it("sends a signed-out visitor to register and remembers the invite", async () => {
    mocks.session.phase = "anonymous";

    await renderInvite("/invite/abc123");

    expect(window.sessionStorage.getItem(PENDING_INVITE_STORAGE_KEY)).toBe("abc123");
    expect(history.navigations).toEqual([{ kind: "replace", href: "/register" }]);
    expect(mocks.ports.directory.joinInvite).not.toHaveBeenCalled();
  });

  it("waits while the session is still loading", async () => {
    mocks.session.phase = "loading";

    await renderInvite();

    expect(screen.getByRole("region", { name: "Wczytywanie serwera…" })).toBeInTheDocument();
    expect(history.navigations).toEqual([]);
    expect(mocks.ports.directory.joinInvite).not.toHaveBeenCalled();
  });

  it("joins the server, selects it and returns to the app", async () => {
    mocks.ports.directory.joinInvite.mockResolvedValue({ id: "server-7" });

    await renderInvite("/invite/abc123");

    expect(mocks.ports.directory.joinInvite).toHaveBeenCalledExactlyOnceWith("abc123");
    expect(mocks.ports.directory.selectServer).toHaveBeenCalledExactlyOnceWith("server-7");
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

    expect(await screen.findByRole("alert")).toHaveTextContent("Zaproszenie nie działa");
    expect(mocks.ports.directory.selectServer).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Zamknij" }));
    expect(history.navigations).toEqual([{ kind: "replace", href: "/" }]);
  });

  it("explains an address whose code cannot be read, without asking the server", async () => {
    await renderInvite("/invite/%E0%A4%A");

    expect(screen.getByRole("alert")).toHaveTextContent("Zaproszenie nie działa");
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
