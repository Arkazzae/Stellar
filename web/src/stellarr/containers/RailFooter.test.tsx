import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { resetStubs, setStubs, stubs } from "@/test/hookStubs";

import { RailAccount, RailVoiceDock } from "./RailFooter";

vi.mock("@/state/hooks", async () => (await import("@/test/hookStubs")).hooksModule);

beforeEach(resetStubs);

describe("RailVoiceDock", () => {
  beforeEach(() => {
    setStubs({
      voice: {
        call: {
          channelId: "c-salon",
          channelName: "Salon",
          startedAt: Date.now(),
          muted: false,
          deafened: false,
          cameraOn: false,
          cameraPending: false,
          sharing: false,
          latencyMs: 40,
          connectionQuality: "good",
          headline: "connected",
        },
      },
    });
  });

  it("leaves the call through the ui", async () => {
    const user = userEvent.setup();
    render(<RailVoiceDock />);

    await user.click(screen.getByRole("button", { name: "Rozłącz" }));

    expect(stubs.ui.leaveCall).toHaveBeenCalledTimes(1);
  });

  it("stores a choice from the microphone menu in the voice settings", async () => {
    const user = userEvent.setup();
    render(<RailVoiceDock />);

    await user.click(screen.getByRole("button", { name: "Jakość i ustawienia mikrofonu" }));
    await user.click(screen.getByRole("radio", { name: "Speex" }));

    expect(stubs.ports.voiceSettings.update).toHaveBeenCalledWith({
      noiseSuppression: "speex",
    });
  });

  it("folds down to its status line while the call's view is on screen", () => {
    const controls = () => screen.queryByRole("button", { name: "Mikrofon" });
    const views: [Parameters<typeof setStubs>[0]["ui"], boolean][] = [
      [{ space: "server", roomView: "text", inConversationCall: false }, false],
      [{ space: "server", roomView: "voice", inConversationCall: false }, true],
      // In a conversation the channel's stage is not on screen, whatever the
      // server side last showed.
      [{ space: "dm", roomView: "voice", inConversationCall: false }, false],
      [{ space: "dm", roomView: "text", inConversationCall: true }, true],
    ];
    for (const [ui, folded] of views) {
      setStubs({ ui });
      const { unmount } = render(<RailVoiceDock />);
      expect(controls() === null).toBe(folded);
      expect(screen.getByRole("button", { name: "Rozłącz" })).toBeInTheDocument();
      unmount();
    }
  });
});

describe("RailAccount", () => {
  it("shows the status you chose rather than the one the socket implies", () => {
    setStubs({ session: { presence: "dnd" } });
    render(<RailAccount />);

    expect(screen.getByRole("button", { name: "MajaNie przeszkadzać" })).toBeInTheDocument();
  });

  it("holds a placeholder until the signed-in user is known", () => {
    setStubs({ session: { user: null } });
    render(<RailAccount />);

    expect(screen.getByRole("button", { name: /^…/ })).toBeInTheDocument();
  });

  it("opens the account panel when the shell is asked to, with the focus on the availability that is set", async () => {
    setStubs({ session: { presence: "dnd" }, ui: { accountPanelOpen: true } });
    render(<RailAccount />);

    const availability = screen.getByRole("group", { name: "Twoja dostępność" });
    expect(availability).toBeVisible();
    const chosen = within(availability).getByRole("button", { pressed: true });
    expect(chosen).toHaveTextContent("Nie przeszkadzać");
    await waitFor(() => expect(chosen).toHaveFocus());
  });

  it("opens the account settings from the only shortcut beside the user", async () => {
    const user = userEvent.setup();
    render(<RailAccount />);

    await user.click(screen.getByRole("button", { name: "Ustawienia użytkownika" }));

    expect(stubs.ui.openModal).toHaveBeenCalledWith("account");
    expect(screen.queryByRole("button", { name: "Wiadomości prywatne" })).not.toBeInTheDocument();
  });
});
