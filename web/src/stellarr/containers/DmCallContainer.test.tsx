import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_SETTINGS } from "@/lib/voice/settings";
import type { Ports } from "@/ports";
import type { FriendsValue, VoiceValue } from "@/state/hooks";
import { resetStubs, setStubs, stubs } from "@/test/hookStubs";

import { DmCallContainer } from "./DmCallContainer";

vi.mock("@/state/hooks", async () => (await import("@/test/hookStubs")).hooksModule);

const ADA = { id: "u-ada", username: "ada", displayName: "Ada", presence: "online" } as const;

beforeEach(() => {
  resetStubs();
  setStubs({
    ports: {
      voiceSettings: {
        getSnapshot: () => DEFAULT_SETTINGS,
        subscribe: () => () => {},
        update: vi.fn(),
      },
    } as unknown as Partial<Ports>,
    friends: { active: { id: "dm-ada", peer: ADA } } as Partial<FriendsValue>,
    voice: {
      channelId: "dm-ada",
      state: { muted: false, cameraPending: false, connection: "connected" },
      call: { startedAt: Date.now(), headline: "connected", cameraOn: false },
      people: [{ id: ADA.id, displayName: "Ada", initials: "AD", presence: "online", speaking: true }],
      problem: null,
      share: null,
      localShare: false,
      cameraStreams: new Map(),
      heldCameras: new Set(),
      // The voice provider turns the soundboard off for a private call.
      soundboardEnabled: false,
    } as unknown as Partial<VoiceValue>,
  });
});

describe("DmCallContainer", () => {
  it("offers the call's own controls, and not inviting, a window of its own or sounds", async () => {
    const user = userEvent.setup();
    render(<DmCallContainer fullscreenTarget={() => null} />);

    for (const name of ["Mikrofon", "Włącz kamerę", "Udostępnij ekran", "Tryb kinowy", "Pełny ekran"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
    for (const name of ["Zaproś na kanał", "Otwórz w osobnym oknie", "Soundboard"]) {
      expect(screen.queryByRole("button", { name })).toBeNull();
    }

    await user.click(screen.getByRole("button", { name: "Rozłącz" }));
    expect(stubs.ui.leaveCall).toHaveBeenCalledOnce();
  });
});
