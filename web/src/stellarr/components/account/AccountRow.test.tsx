import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { VoiceSeatVM } from "@/components/view-models";
import { VoiceSeatContext } from "@/components/voice/useVoiceSeat";

import { AccountRow, type AccountRowProps } from "./AccountRow";

function setup(overrides: Partial<AccountRowProps> = {}, seat: VoiceSeatVM | null = null) {
  const props: AccountRowProps = {
    user: {
      id: "u-maja-9",
      displayName: "Maja",
      initials: "MA",
      presence: "online",
    },
    onOpenSettings: vi.fn(),
    panel: {
      presence: "online",
      customStatus: "",
      onSetPresence: vi.fn(),
      onSaveCustomStatus: vi.fn(),
      onLogout: vi.fn(),
    },
    ...overrides,
  };

  render(
    <VoiceSeatContext.Provider value={{ get: () => seat, subscribe: () => () => undefined }}>
      <AccountRow {...props} />
    </VoiceSeatContext.Provider>,
  );
}

describe("AccountRow", () => {
  it("says, in a call, that your microphone is off, and rings you while you are heard", () => {
    setup({}, { speaking: false, muted: true, sharing: false, camera: false });

    const trigger = screen.getByRole("button", { name: /Maja/ });
    expect(within(trigger).getByText("Mikrofon wyłączony")).toHaveClass("mic-off");
    expect(trigger.querySelector(".acc-av")).toHaveClass("muted");
  });

  it("rings your avatar while you are heard in a call", () => {
    setup({}, { speaking: true, muted: false, sharing: false, camera: false });

    const trigger = screen.getByRole("button", { name: "MajaOnline" });
    expect(trigger.querySelector(".av")).toHaveClass("speaking");
    expect(trigger.querySelector(".acc-mic")).toBeNull();
  });

  it("shows a short presence label under the name", () => {
    setup();

    const trigger = screen.getByRole("button", { name: "MajaOnline" });
    expect(within(trigger).getByText("Online")).toBeVisible();
    expect(screen.queryByText("Jesteś i można pisać.")).toBeNull();
  });

  it("shows the custom status under the name when it is set", () => {
    setup({
      panel: {
        presence: "online",
        customStatus: "🎧 na słuchawkach",
        onSetPresence: vi.fn(),
        onSaveCustomStatus: vi.fn(),
        onLogout: vi.fn(),
      },
    });

    const trigger = screen.getByRole("button", {
      name: "Maja🎧 na słuchawkach",
    });
    expect(within(trigger).getByText("🎧 na słuchawkach")).toBeVisible();
  });
});
