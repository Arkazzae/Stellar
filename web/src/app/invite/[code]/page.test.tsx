import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { PENDING_INVITE_STORAGE_KEY } from "@/lib/invites";
import { translateMessage, type MessageKey } from "@/i18n";
import type { SessionPhase } from "@/state/hooks";

import InvitePage, { generateStaticParams } from "./page";

const mocks = vi.hoisted(() => ({
  session: { phase: "ready" as SessionPhase, reload: vi.fn() },
  ports: { directory: { joinInvite: vi.fn(), selectServer: vi.fn() } },
  router: { replace: vi.fn() },
  pathname: "/invite/abc123",
}));

vi.mock("next/navigation", () => ({
  useRouter: () => mocks.router,
  usePathname: () => mocks.pathname,
}));
vi.mock("@/state/hooks", () => ({
  useSession: () => mocks.session,
  usePorts: () => mocks.ports,
  useLanguage: () => ({ t: (key: MessageKey) => translateMessage("pl", key) }),
}));

async function renderInvite(pathname = "/invite/abc123") {
  mocks.pathname = pathname;
  await act(async () => {
    render(<InvitePage />);
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.session.phase = "ready";
  window.sessionStorage.clear();
});

describe("InvitePage", () => {
  it("is rendered ahead of time once, for a code no invite has", () => {
    expect(generateStaticParams()).toEqual([{ code: "_" }]);
  });

  it("sends a signed-out visitor to register and remembers the invite", async () => {
    mocks.session.phase = "anonymous";

    await renderInvite("/invite/abc123");

    expect(window.sessionStorage.getItem(PENDING_INVITE_STORAGE_KEY)).toBe("abc123");
    expect(mocks.router.replace).toHaveBeenCalledExactlyOnceWith("/register");
    expect(mocks.ports.directory.joinInvite).not.toHaveBeenCalled();
  });

  it("waits while the session is still loading", async () => {
    mocks.session.phase = "loading";

    await renderInvite();

    expect(screen.getByRole("region", { name: "Wczytywanie serwera…" })).toBeInTheDocument();
    expect(mocks.router.replace).not.toHaveBeenCalled();
    expect(mocks.ports.directory.joinInvite).not.toHaveBeenCalled();
  });

  it("joins the server, selects it and returns to the app", async () => {
    mocks.ports.directory.joinInvite.mockResolvedValue({ id: "server-7" });

    await renderInvite("/invite/abc123");

    expect(mocks.ports.directory.joinInvite).toHaveBeenCalledExactlyOnceWith("abc123");
    expect(mocks.ports.directory.selectServer).toHaveBeenCalledExactlyOnceWith("server-7");
    expect(mocks.router.replace).toHaveBeenCalledExactlyOnceWith("/");
    expect(mocks.session.reload).toHaveBeenCalledOnce();
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
    expect(mocks.router.replace).toHaveBeenCalledExactlyOnceWith("/");
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
      render(<InvitePage />).unmount();
    });

    await act(async () => finishJoin({ id: "server-7" }));

    expect(mocks.ports.directory.selectServer).not.toHaveBeenCalled();
    expect(mocks.router.replace).not.toHaveBeenCalled();
  });
});
