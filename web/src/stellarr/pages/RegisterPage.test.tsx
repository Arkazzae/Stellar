import { Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { rememberPendingInvite } from "@/lib/auth/invites";
import { translateMessage, type MessageKey } from "@/i18n";
import { routes } from "@/routes";
import type { SessionPhase } from "@/state/hooks";

import RegistrationPage from "./RegisterPage";

const mocks = vi.hoisted(() => ({
  session: { phase: "anonymous" as SessionPhase, reload: vi.fn() },
  ports: { session: { register: vi.fn(), needsServerSetup: vi.fn() } },
}));

vi.mock("@/state/hooks", () => ({
  useSession: () => mocks.session,
  usePorts: () => mocks.ports,
  useLanguage: () => ({ t: (key: MessageKey) => translateMessage("pl", key) }),
}));

let history: MemoryHistory;

function renderRegistration() {
  history = createMemoryHistory("/register");
  return render(
    <Router routes={routes} history={history}>
      <RegistrationPage />
    </Router>,
  );
}

async function fillAndSubmit() {
  const user = userEvent.setup();
  await user.type(await screen.findByRole("textbox", { name: "Nazwa widoczna" }), "Maja");
  await user.type(screen.getByRole("textbox", { name: "Login" }), "maja");
  await user.type(screen.getByRole("textbox", { name: "E-mail" }), "maja@example.com");
  await user.type(screen.getByLabelText("Hasło"), "haslo123");
  await user.type(screen.getByLabelText("Powtórz hasło"), "haslo123");
  await user.click(screen.getByRole("button", { name: /Załóż konto|Utwórz konto i serwer/ }));
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.session.phase = "anonymous";
  window.sessionStorage.clear();
  mocks.ports.session.needsServerSetup.mockResolvedValue(false);
});

describe("RegistrationPage", () => {
  it("configures the first server before creating its owner", async () => {
    mocks.ports.session.needsServerSetup.mockResolvedValue(true);
    mocks.ports.session.register.mockResolvedValue({});
    const user = userEvent.setup();
    renderRegistration();
    await user.type(await screen.findByRole("textbox", { name: "Nazwa serwera" }), "Ekipa");
    await user.click(screen.getByRole("button", { name: "Dalej: konto właściciela" }));
    expect(mocks.ports.session.register).not.toHaveBeenCalled();
    await fillAndSubmit();
    expect(mocks.ports.session.register).toHaveBeenCalledWith(expect.objectContaining({
      initialServer: { name: "Ekipa", description: "", textChannelName: "ogólny", voiceChannelName: "rozmowa" },
    }));
  });

  it("retries a failed configuration check", async () => {
    mocks.ports.session.needsServerSetup.mockRejectedValueOnce(new Error("offline")).mockResolvedValue(true);
    renderRegistration();
    expect(await screen.findByRole("alert")).toHaveTextContent("Nie udało się sprawdzić");
    await userEvent.setup().click(screen.getByRole("button", { name: "Spróbuj ponownie" }));
    expect(await screen.findByRole("textbox", { name: "Nazwa serwera" })).toBeVisible();
  });

  it("registers the account and asks the session to start over", async () => {
    mocks.ports.session.register.mockResolvedValue({ kind: "signed-in", user: { id: "u-1", username: "maja", displayName: "Maja", presence: "online" } });
    renderRegistration();

    await fillAndSubmit();

    expect(mocks.ports.session.register).toHaveBeenCalledExactlyOnceWith({
      displayName: "Maja",
      username: "maja",
      email: "maja@example.com",
      password: "haslo123",
    });
    expect(mocks.session.reload).toHaveBeenCalledOnce();
  });

  it("registers with the invite the visitor arrived with and keeps it for the redirect", async () => {
    mocks.ports.session.register.mockResolvedValue({ kind: "signed-in", user: { id: "u-1", username: "maja", displayName: "Maja", presence: "online" } });
    rememberPendingInvite("abc123");
    renderRegistration();

    await fillAndSubmit();

    expect(mocks.ports.session.register).toHaveBeenCalledWith(
      expect.objectContaining({ inviteCode: "abc123" }),
    );
    expect(window.sessionStorage.getItem("stellarr.pending-invite")).toBe("abc123");
  });

  it("leaves a required second-step setup to the sign-in page, without starting the app", async () => {
    mocks.ports.session.register.mockResolvedValue({
      kind: "challenge",
      challenge: { enrollment: true, challengeId: "c-1", purpose: "enrollment", methods: [] },
    });
    renderRegistration();

    await fillAndSubmit();

    expect(history.navigations).toEqual([{ kind: "replace", href: "/login" }]);
    expect(mocks.session.reload).not.toHaveBeenCalled();
  });

  it("says the account could not be created when the server refuses", async () => {
    mocks.ports.session.register.mockRejectedValue(new Error("taken"));
    renderRegistration();

    await fillAndSubmit();

    expect(await screen.findByRole("alert")).toHaveTextContent("Nie udało się założyć konta.");
    expect(mocks.session.reload).not.toHaveBeenCalled();
  });

  it("keeps the focus on the heading when the form replaces the frame that waited for the server", async () => {
    let answer: (needed: boolean) => void = () => undefined;
    mocks.ports.session.needsServerSetup.mockReturnValue(new Promise<boolean>((resolve) => (answer = resolve)));
    renderRegistration();
    // Where the router puts the focus after a link led here.
    screen.getByRole("heading", { name: "Załóż konto" }).focus();

    await act(async () => answer(false));

    const heading = screen.getByRole("heading", { name: "Załóż konto" });
    expect(await screen.findByRole("textbox", { name: "Login" })).toBeInTheDocument();
    expect(heading).toHaveFocus();
  });

  it("gives the account step's heading the focus that the button of the server step had", async () => {
    mocks.ports.session.needsServerSetup.mockResolvedValue(true);
    const user = userEvent.setup();
    renderRegistration();
    await user.type(await screen.findByRole("textbox", { name: "Nazwa serwera" }), "Ekipa");

    await user.click(screen.getByRole("button", { name: "Dalej: konto właściciela" }));

    expect(screen.getByRole("heading", { name: "Załóż konto" })).toHaveFocus();
  });

  it("moves on to the invite once the new session is ready", () => {
    mocks.session.phase = "ready";
    rememberPendingInvite("abc123");

    renderRegistration();

    expect(history.navigations).toEqual([{ kind: "replace", href: "/invite/abc123" }]);
  });
});
