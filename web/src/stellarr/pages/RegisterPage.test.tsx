import { Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/lib/api/errors";
import { pendingInvite, rememberPendingInvite } from "@/lib/auth/invites";
import { routes } from "@/routes";
import type { InvitePreview, PendingIdentity } from "@/ports";
import type { SessionPhase } from "@/state/hooks";
import { polishAsRead } from "@/test/translators";

import RegistrationPage from "./RegisterPage";

const mocks = vi.hoisted(() => ({
  session: { phase: "anonymous" as SessionPhase, reload: vi.fn() },
  ports: {
    session: {
      register: vi.fn(),
      invitePreview: vi.fn(),
      instanceName: vi.fn(),
      signInProviders: vi.fn(),
      pendingIdentity: vi.fn(),
    },
  },
}));

vi.mock("@/state/hooks", () => ({
  useSession: () => mocks.session,
  usePorts: () => mocks.ports,
  useLanguage: () => ({ t }),
}));

const t = polishAsRead;

const PREVIEW: InvitePreview = {
  server: { id: "s-1", name: "Stellarr Dev", description: "", iconUrl: null },
  channel: null,
  inviter: { displayName: "Ada", avatarColor: "oklch(0.58 0.14 24)", avatarUrl: null },
  expiresAt: null,
};

let history: MemoryHistory;

async function renderRegistration(path = "/register") {
  history = createMemoryHistory(path);
  await act(async () => {
    render(
      <Router routes={routes} history={history}>
        <RegistrationPage />
      </Router>,
    );
  });
}

async function fillAndSubmit() {
  const user = userEvent.setup();
  await user.type(await screen.findByRole("textbox", { name: "Nazwa widoczna" }), "Maja");
  await user.type(screen.getByRole("textbox", { name: "E-mail" }), "maja@example.com");
  await user.type(screen.getByLabelText("Hasło"), "haslo123");
  await user.click(screen.getByRole("button", { name: t("auth.register.submit") }));
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.session.phase = "anonymous";
  window.sessionStorage.clear();
  // A session port of its own, so the providers asked once per port are asked again.
  mocks.ports.session = { ...mocks.ports.session };
  mocks.ports.session.invitePreview.mockResolvedValue(PREVIEW);
  mocks.ports.session.instanceName.mockResolvedValue("Stellarr Dev");
  mocks.ports.session.signInProviders.mockResolvedValue([]);
  mocks.ports.session.pendingIdentity.mockResolvedValue(null);
});

describe("RegistrationPage", () => {
  it("asks for an invite link instead of a form the server would refuse, and opens the invite", async () => {
    const user = userEvent.setup();
    await renderRegistration();

    expect(screen.queryByRole("textbox", { name: "Nazwa widoczna" })).toBeNull();
    await user.type(screen.getByRole("textbox", { name: t("auth.invite.linkLabel") }), "chat.example/invite/k7qm2xwp");
    await user.click(screen.getByRole("button", { name: t("auth.invite.open") }));

    expect(history.navigations.at(-1)).toEqual({ kind: "push", href: "/invite/k7qm2xwp" });
    expect(mocks.ports.session.register).not.toHaveBeenCalled();
  });

  it("registers with the invite the visitor arrived with, says where it leads, and keeps it for the redirect", async () => {
    rememberPendingInvite("abc123");
    mocks.ports.session.register.mockResolvedValue({ kind: "signed-in", user: {} });
    await renderRegistration();

    expect(
      await screen.findByText(t("auth.register.descriptionInvite", { server: "Stellarr Dev", inviter: "Ada" })),
    ).toBeVisible();
    expect(mocks.ports.session.invitePreview).toHaveBeenCalledExactlyOnceWith("abc123");
    await fillAndSubmit();

    expect(mocks.ports.session.register).toHaveBeenCalledExactlyOnceWith({
      displayName: "Maja",
      username: "maja",
      email: "maja@example.com",
      password: "haslo123",
      inviteCode: "abc123",
    });
    expect(mocks.session.reload).toHaveBeenCalledOnce();
    // The redirect after the session takes the invite, not the registration.
    expect(pendingInvite()).toBe("abc123");
  });

  it("leaves a required second-step setup to the sign-in page, without starting the app", async () => {
    rememberPendingInvite("abc123");
    mocks.ports.session.register.mockResolvedValue({ kind: "challenge", challenge: {} });
    await renderRegistration();

    await fillAndSubmit();

    expect(history.navigations.at(-1)).toEqual({ kind: "replace", href: "/login" });
    expect(mocks.session.reload).not.toHaveBeenCalled();
  });

  it("names the field the server refused", async () => {
    rememberPendingInvite("abc123");
    mocks.ports.session.register.mockRejectedValue(
      new ApiError("invalid", "400", 400, { fields: { username: "Login zajęty." } }),
    );
    await renderRegistration();

    await fillAndSubmit();

    expect(await screen.findByRole("alert")).toHaveTextContent(t("auth.register.usernameTaken"));
    expect(screen.getByRole("textbox", { name: "Login" })).toHaveAttribute("aria-invalid", "true");
  });

  it("forgets an invite that no longer works and asks for a new link", async () => {
    rememberPendingInvite("gone");
    mocks.ports.session.invitePreview.mockRejectedValue(new ApiError("not-found", "404", 404));
    await renderRegistration();

    expect(await screen.findByRole("alert")).toHaveTextContent(t("auth.register.inviteRefused"));
    expect(screen.getByRole("textbox", { name: t("auth.invite.linkLabel") })).toBeVisible();
    expect(pendingInvite()).toBeNull();
  });

  it("moves on to the invite once the new session is ready", async () => {
    rememberPendingInvite("abc123");
    mocks.session.phase = "ready";

    await renderRegistration();

    expect(history.navigations).toEqual([{ kind: "replace", href: "/invite/abc123" }]);
  });

  it("offers the providers that may create accounts, each coming back to the account made through it", async () => {
    rememberPendingInvite("abc123");
    mocks.ports.session.signInProviders.mockResolvedValue([
      { id: "default", displayName: "Authentik", loginUrl: "/api/auth/oidc/login", signUp: true },
      { id: "work", displayName: "Praca", loginUrl: "/api/auth/oidc/work/login", signUp: false },
    ]);
    await renderRegistration();

    expect(
      await screen.findByRole("link", { name: t("auth.register.withProvider", { provider: "Authentik" }) }),
    ).toHaveAttribute("href", "/api/auth/oidc/login?returnUrl=%2Fregister%3Fwith%3Dprovider");
    expect(screen.queryByRole("link", { name: t("auth.register.withProvider", { provider: "Praca" }) })).toBeNull();
  });
});

describe("RegistrationPage through a provider", () => {
  const OLA: PendingIdentity = {
    provider: { id: "default", displayName: "Authentik" },
    displayName: "Ola Nowak",
    email: "ola@example.test",
    username: "ola.nowak",
    signUp: true,
  };

  beforeEach(() => {
    rememberPendingInvite("k7qm");
    mocks.ports.session.pendingIdentity.mockResolvedValue(OLA);
  });

  it("makes the account from what the provider said, with the invite and no password", async () => {
    mocks.ports.session.register.mockResolvedValue({ kind: "signed-in", user: {} });
    await renderRegistration("/register?with=provider");

    expect(await screen.findByRole("textbox", { name: "Nazwa widoczna" })).toHaveValue("Ola Nowak");
    expect(screen.queryByLabelText("Hasło")).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: t("auth.register.submit") }));

    expect(mocks.ports.session.register).toHaveBeenCalledExactlyOnceWith({
      displayName: "Ola Nowak",
      username: "ola.nowak",
      email: "ola@example.test",
      external: true,
      inviteCode: "k7qm",
    });
    expect(mocks.session.reload).toHaveBeenCalledOnce();
  });

  it("says to sign in at the provider again when its confirmation ran out", async () => {
    mocks.ports.session.register.mockRejectedValue(new ApiError("invalid", "400", 400));
    await renderRegistration("/register?with=provider");

    await userEvent.setup().click(await screen.findByRole("button", { name: t("auth.register.submit") }));

    expect(await screen.findByRole("alert")).toHaveTextContent(t("auth.providerSignUp.expired"));
  });
});
