import { Router } from "@stellarr/router";
import { createMemoryHistory, type MemoryHistory } from "@stellarr/router/testing";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/lib/api/errors";
import { routes } from "@/routes";
import type { SessionPhase } from "@/state/hooks";
import { polishAsRead as t } from "@/test/translators";

import SetupPage from "./SetupPage";

const mocks = vi.hoisted(() => ({
  session: { phase: "anonymous" as SessionPhase, reload: vi.fn() },
  setup: { setupRequired: true as boolean | null, failed: false, retry: vi.fn(), markSetUp: vi.fn() },
  ports: { session: { register: vi.fn() }, admin: { createInvite: vi.fn() } },
}));

vi.mock("@/state/SessionProvider", () => ({
  useSessionContext: () => mocks.session,
}));
vi.mock("@/state/InstanceSetupProvider", () => ({
  useInstanceSetupContext: () => mocks.setup,
}));
vi.mock("@/state/PortsProvider", () => ({
  usePorts: () => mocks.ports,
}));
vi.mock("@/state/LanguageProvider", () => ({
  useLanguage: () => ({ t }),
}));

let history: MemoryHistory;

async function renderSetup() {
  history = createMemoryHistory("/setup");
  await act(async () => {
    render(
      <Router routes={routes} history={history}>
        <SetupPage />
      </Router>,
    );
  });
}

const textbox = (name: string) => screen.getByRole("textbox", { name });

async function fillOwner(user: ReturnType<typeof userEvent.setup>, code = "K7QM-2XWP-9HDA") {
  await user.type(textbox(t("auth.setup.code")), code);
  await user.type(textbox(t("auth.register.displayName")), "Łukasz Żółć");
  await user.type(textbox(t("auth.email")), "owner@example.com");
  await user.type(screen.getByLabelText(t("auth.password")), "zielony.most.42");
  await user.click(screen.getByRole("button", { name: t("auth.setup.continue") }));
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.session.phase = "anonymous";
  mocks.setup.setupRequired = true;
  mocks.setup.failed = false;
  mocks.ports.admin.createInvite.mockResolvedValue({ id: "i-1", url: "https://127.0.0.1:8443/invite/k7qm2xwp" });
});

describe("SetupPage", () => {
  it("leaves for the sign-in page once the instance has an owner, and for home with a session", async () => {
    mocks.setup.setupRequired = false;
    await renderSetup();
    expect(history.navigations).toEqual([{ kind: "replace", href: "/login" }]);

    mocks.session.phase = "ready";
    mocks.setup.setupRequired = null;
    await renderSetup();
    expect(history.navigations).toEqual([{ kind: "replace", href: "/" }]);
  });

  it("makes the owner and the server in one registration, then offers a link to invite people", async () => {
    const user = userEvent.setup();
    mocks.ports.session.register.mockResolvedValue({ kind: "signed-in", user: {} });
    await renderSetup();

    // The code from the log comes first; nothing goes on without it.
    await user.click(screen.getByRole("button", { name: t("auth.setup.continue") }));
    expect(screen.getByRole("alert")).toHaveTextContent(t("auth.setup.codeMissing"));
    await fillOwner(user);
    await user.type(await screen.findByRole("textbox", { name: t("auth.setup.name") }), " Nocna ekipa ");
    await user.click(screen.getByRole("button", { name: t("auth.setup.create") }));

    expect(mocks.ports.session.register).toHaveBeenCalledExactlyOnceWith({
      displayName: "Łukasz Żółć",
      username: "lukasz.zolc",
      email: "owner@example.com",
      password: "zielony.most.42",
      setupCode: "K7QM-2XWP-9HDA",
      initialServer: {
        name: "Nocna ekipa",
        description: "",
        textChannelName: t("auth.setup.defaultText"),
        voiceChannelName: t("auth.setup.defaultVoice"),
      },
    });
    expect(mocks.setup.markSetUp).toHaveBeenCalledOnce();
    expect(mocks.ports.admin.createInvite).toHaveBeenCalledOnce();
    expect(await screen.findByText("https://127.0.0.1:8443/invite/k7qm2xwp")).toBeVisible();
    expect(screen.getByText(t("auth.setup.localInvite"))).toBeVisible();
    expect(screen.getByRole("link", { name: t("auth.setup.configureAccess") })).toHaveAttribute("rel", "noreferrer");
    // Set up and anonymous to the provider, the page still shows its last step.
    expect(history.navigations).toEqual([]);

    await user.click(screen.getByRole("button", { name: t("auth.setup.goToServer") }));
    expect(mocks.session.reload).toHaveBeenCalledOnce();
    expect(history.navigations).toEqual([{ kind: "replace", href: "/" }]);
  });

  it("sends a refused setup code back to the first step, onto its field", async () => {
    const user = userEvent.setup();
    mocks.ports.session.register.mockRejectedValue(
      new ApiError("invalid", "400", 400, { fields: { setupCode: "Kod jest nieprawidłowy." } }),
    );
    await renderSetup();

    await fillOwner(user, "K7QM-2XWP-0000");
    await user.type(await screen.findByRole("textbox", { name: t("auth.setup.name") }), "Nocna ekipa");
    await user.click(screen.getByRole("button", { name: t("auth.setup.create") }));

    expect(await screen.findByRole("heading", { name: t("auth.setup.ownerTitle") })).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent(t("auth.setup.codeRefused"));
    expect(textbox(t("auth.setup.code"))).toHaveAttribute("aria-invalid", "true");
    expect(textbox(t("auth.setup.code"))).toHaveFocus();
    // What was typed is still there.
    expect(textbox(t("auth.register.displayName"))).toHaveValue("Łukasz Żółć");
    expect(mocks.setup.markSetUp).not.toHaveBeenCalled();
  });

  it("goes on at the sign-in page when the instance asks the owner for a second step", async () => {
    const user = userEvent.setup();
    mocks.ports.session.register.mockResolvedValue({ kind: "challenge", challenge: {} });
    await renderSetup();

    await fillOwner(user);
    await user.type(await screen.findByRole("textbox", { name: t("auth.setup.name") }), "Nocna ekipa");
    await user.click(screen.getByRole("button", { name: t("auth.setup.create") }));

    expect(mocks.setup.markSetUp).toHaveBeenCalledOnce();
    expect(history.navigations).toEqual([{ kind: "replace", href: "/login" }]);
    expect(mocks.ports.admin.createInvite).not.toHaveBeenCalled();
    expect(mocks.session.reload).not.toHaveBeenCalled();
  });
});
