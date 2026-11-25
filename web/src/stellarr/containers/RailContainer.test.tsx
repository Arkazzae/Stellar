import { act, render, screen, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import type { PersonVM, VoiceSeatVM } from "@/components/view-models";
import { VoiceSeatContext } from "@/components/voice/useVoiceSeat";
import { resetStubs, setStubs, stubs } from "@/test/hookStubs";

import { RailContainer } from "./RailContainer";

vi.mock("@/state/hooks", async () => (await import("@/test/hookStubs")).hooksModule);

const MAJA: PersonVM = {
  id: "u-maja",
  displayName: "Maja",
  initials: "MA",
  presence: "online",
};

const TEXT_CHANNELS = [
  { id: "c-general", name: "ogólny" },
  { id: "c-dev", name: "dev" },
];

function renderRail(seats: ReadonlyMap<string, VoiceSeatVM> = new Map()) {
  return render(
    <VoiceSeatContext.Provider
      value={{ get: (userId) => seats.get(userId) ?? null, subscribe: () => () => undefined }}
    >
      <ConfirmProvider>
        <RailContainer />
      </ConfirmProvider>
    </VoiceSeatContext.Provider>,
  );
}

/** The whole row of a channel, which is where its badges and menu live. */
function rowOf(name: string): HTMLElement {
  const row = screen.getByText(name).closest<HTMLElement>(".channel");
  if (row === null) throw new Error(`no rail row for ${name}`);
  return row;
}

async function openChannelMenu(user: UserEvent, name: string) {
  await user.pointer({ keys: "[MouseRight]", target: rowOf(name) });
  return await screen.findByRole("menu");
}

beforeEach(() => {
  resetStubs();
  setStubs({
    directory: {
      textChannels: TEXT_CHANNELS,
      voiceChannels: [{ id: "c-salon", name: "Salon", occupants: [MAJA] }],
    },
  });
});

describe("RailContainer", () => {
  it("puts the unread count the chat holds on each text channel", () => {
    setStubs({ chat: { unreadByChannel: { "c-dev": 4 } } });
    renderRail();

    expect(within(rowOf("dev")).getByLabelText("Nieprzeczytane: 4")).toBeInTheDocument();
    expect(within(rowOf("ogólny")).queryByLabelText(/Nieprzeczytane/)).toBeNull();
  });

  it("marks only the channels the account has muted", () => {
    setStubs({
      account: {
        preferences: { ...stubs.account.preferences, mutedChannelIds: ["c-dev"] },
      },
    });
    renderRail();

    expect(within(rowOf("dev")).getByLabelText("Powiadomienia wyciszone")).toBeInTheDocument();
    expect(within(rowOf("ogólny")).queryByLabelText("Powiadomienia wyciszone")).toBeNull();
  });

  it("marks who shares a screen and who has a camera on, from the server's list", () => {
    setStubs({
      directory: {
        voiceChannels: [
          { id: "c-salon", name: "Salon", occupants: [{ ...MAJA, sharing: true, camera: true }] },
        ],
      },
    });
    renderRail();

    const row = screen
      .getAllByText("Maja")
      .map((name) => name.closest(".voice-occupant"))
      .find((item) => item !== null)!;
    expect(row).toHaveTextContent("Na żywo");
    expect(row.querySelector(".occ-camera")).toHaveAttribute("aria-label", "Kamera włączona");
  });

  it("shows who is speaking in the call, whose word on the microphone outranks the server's", () => {
    setStubs({
      directory: {
        voiceChannels: [{ id: "c-salon", name: "Salon", occupants: [{ ...MAJA, muted: true }] }],
      },
    });
    renderRail(new Map([[MAJA.id, { speaking: true, muted: false, sharing: false, camera: false }]]));

    // Her name is also on the account row, which is never a speaker.
    const rows = screen
      .getAllByText("Maja")
      .map((name) => name.closest(".voice-occupant"))
      .filter((row) => row !== null);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toHaveClass("speaking");
    expect(rows[0]).not.toHaveClass("muted");
  });

  it("keeps an open person menu's strip in step with the call", async () => {
    let volume = 100;
    const listeners = new Set<() => void>();
    const ola: PersonVM = { id: "u-ola", displayName: "Ola", initials: "OL", presence: "online" };
    setStubs({
      directory: { voiceChannels: [{ id: "c-salon", name: "Salon", occupants: [MAJA, ola] }] },
      voiceLive: {
        participantVolumeOf: () => volume,
        subscribe: (listener) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        },
      },
    });
    const user = userEvent.setup();
    renderRail();

    await user.pointer({ keys: "[MouseRight]", target: screen.getByText("Ola") });
    const slider = await screen.findByRole("slider");
    expect(slider).toHaveValue("100");

    volume = 150;
    act(() => {
      for (const listener of listeners) listener();
    });

    expect(slider).toHaveValue("150");
  });

  it("closes the open thread before it selects a text channel", async () => {
    const user = userEvent.setup();
    renderRail();

    await user.click(screen.getByRole("button", { name: "ogólny" }));

    expect(stubs.chat.closeThread).toHaveBeenCalledTimes(1);
    expect(stubs.ui.selectTextChannel).toHaveBeenCalledWith("c-general");
    expect(vi.mocked(stubs.chat.closeThread).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(stubs.ui.selectTextChannel).mock.invocationCallOrder[0] ?? 0,
    );
  });

  it("joins a voice channel through the ui", async () => {
    const user = userEvent.setup();
    renderRail();

    await user.click(screen.getByRole("button", { name: /^Salon/ }));

    expect(stubs.ui.joinVoiceChannel).toHaveBeenCalledWith("c-salon");
  });

  it("opens a thread from the summary the chat holds for it", async () => {
    const summary = {
      channelId: "c-general",
      replyCount: 2,
      lastReplyAt: "2026-01-01T00:00:00.000Z",
      root: { id: "m-1", authorId: "u-maja", authorName: "Maja", preview: "Kto robi release?" },
    };
    setStubs({ chat: { threadsByChannel: { "c-general": [summary] } } });
    const user = userEvent.setup();
    renderRail();

    await user.click(screen.getByRole("button", { name: "ogólny" }));
    await user.click(screen.getByRole("button", { name: /Kto robi release/ }));

    expect(stubs.chat.openThread).toHaveBeenCalledWith(summary);
  });

  describe("channel menu", () => {
    it("adds a channel to the muted list and takes it off again", async () => {
      setStubs({
        account: {
          preferences: { ...stubs.account.preferences, mutedChannelIds: ["c-dev"] },
        },
      });
      const user = userEvent.setup();
      renderRail();

      const first = await openChannelMenu(user, "ogólny");
      await user.click(
        within(first).getByRole("menuitem", { name: "Wycisz powiadomienia kanału" }),
      );
      expect(stubs.account.setPreferences).toHaveBeenLastCalledWith({
        mutedChannelIds: ["c-dev", "c-general"],
      });

      const second = await openChannelMenu(user, "dev");
      await user.click(
        within(second).getByRole("menuitem", { name: "Włącz powiadomienia kanału" }),
      );
      expect(stubs.account.setPreferences).toHaveBeenLastCalledWith({
        mutedChannelIds: [],
      });
    });

    it("offers deletion only to a manager on a server that can delete", async () => {
      setStubs({ directory: { canManageChannels: true } });
      const { rerender } = renderRail();
      const user = userEvent.setup();

      await user.pointer({ keys: "[MouseRight]", target: rowOf("dev") });
      expect(screen.queryByRole("menuitem", { name: "Usuń kanał" })).toBeNull();
      await user.keyboard("{Escape}");

      setStubs({
        ports: { admin: { ...stubs.ports.admin, supported: { ...stubs.ports.admin.supported, channels: true } } },
      });
      rerender(
        <ConfirmProvider>
          <RailContainer />
        </ConfirmProvider>,
      );
      await user.pointer({ keys: "[MouseRight]", target: rowOf("dev") });
      expect(await screen.findByRole("menuitem", { name: "Usuń kanał" })).toBeInTheDocument();
    });
  });

  it("leaves the rail inert while it is a closed drawer", () => {
    window.innerWidth = 500;
    try {
      renderRail();
      expect(rowOf("dev").closest("[inert]")).not.toBeNull();
    } finally {
      window.innerWidth = 1024;
    }
  });
});
