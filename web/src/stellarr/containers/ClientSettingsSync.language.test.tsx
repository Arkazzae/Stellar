import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { VoiceSettingsStore } from "@/lib/voice/settings";
import type { ClientSettingsSnapshot } from "@/ports";

/**
 * The account's language reaching a page that has not fetched its catalog:
 * an English device signing in to an account kept in Polish. The page
 * switches once the Polish catalog arrives, and nothing it applied from the
 * account goes back to it, before, after, or when the catalog never comes.
 */

const world = vi.hoisted(() => ({ ports: null as unknown }));

vi.mock("@/state/PortsProvider", () => ({
  usePorts: () => world.ports,
  useRuntime: () => ({}),
}));
vi.mock("@/state/AccountProvider", () => ({
  useAccountContext: () => ({ account: { id: "u1" } }),
}));
vi.mock("@/state/UiProvider", () => ({
  useUiContext: () => ({ leaveCall: vi.fn() }),
}));
vi.mock("@/state/VoiceProvider", () => ({
  useVoiceContext: () => ({
    channelId: null,
    toggleMute: vi.fn(),
    toggleDeafen: vi.fn(),
    toggleCamera: vi.fn(),
  }),
}));

const ACCOUNT: ClientSettingsSnapshot = {
  appearance: null,
  language: { locale: "pl", chatTranslationEnabled: false, chatTranslationTargetLanguage: "en" },
  keybinds: null,
  voice: null,
  privacy: null,
};

const SAVE_WINDOW_MS = 700;

let release: () => void = () => undefined;
let save: ReturnType<typeof vi.fn>;

async function signInOnEnglishDevice() {
  vi.resetModules();
  const i18n = await import("@/i18n");
  await i18n.loadCatalog("en");
  const { LanguageProvider, useLanguage } = await import("@/state/LanguageProvider");
  const { ThemeProvider } = await import("@/state/ThemeProvider");
  const { KeybindProvider } = await import("@/state/KeybindProvider");
  const { ClientSettingsSync } = await import("./ClientSettingsSync");
  /** Picks Polish the way the language pane does. */
  function PickPolish() {
    const { update } = useLanguage();
    return (
      <button type="button" onClick={() => update({ locale: "pl" })}>
        polski
      </button>
    );
  }
  const view = render(
    <LanguageProvider initialLocale="en">
      <ThemeProvider>
        <KeybindProvider>
          <ClientSettingsSync />
          <PickPolish />
        </KeybindProvider>
      </ThemeProvider>
    </LanguageProvider>,
  );
  return { pickPolish: () => view.getByRole("button", { name: "polski" }).click() };
}

const storedLocale = () =>
  (JSON.parse(window.localStorage.getItem("stellarr_language_settings") ?? "{}") as { locale?: string }).locale;

async function afterSaveWindow() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, SAVE_WINDOW_MS));
  });
}

beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.lang = "en";
  vi.spyOn(window, "matchMedia").mockImplementation(
    (query) =>
      ({
        matches: false,
        media: query,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
      }) as unknown as MediaQueryList,
  );
  save = vi.fn(async () => undefined);
  world.ports = {
    account: { loadClientSettings: vi.fn(async () => ACCOUNT), saveClientSettings: save },
    voiceSettings: new VoiceSettingsStore(null),
  };
});

afterEach(() => {
  vi.doUnmock("@/i18n/messages/pl");
});

describe("the account's language on a page without its catalog", () => {
  it("switches once the catalog arrives and sends nothing back", async () => {
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    vi.doMock("@/i18n/messages/pl", async () => {
      await gate;
      return await vi.importActual<object>("@/i18n/messages/pl");
    });
    await signInOnEnglishDevice();

    await waitFor(() => expect(storedLocale()).toBe("pl"));
    expect(document.documentElement.lang).toBe("en");
    await afterSaveWindow();
    expect(save).not.toHaveBeenCalled();

    await act(async () => release());
    await waitFor(() => expect(document.documentElement.lang).toBe("pl"));
    await afterSaveWindow();
    expect(save).not.toHaveBeenCalled();
  });

  it("sends nothing back when the catalog never comes", async () => {
    vi.doMock("@/i18n/messages/pl", () => {
      throw new Error("Failed to fetch dynamically imported module");
    });
    await signInOnEnglishDevice();

    await waitFor(() => expect(storedLocale()).toBe("pl"));
    await afterSaveWindow();

    expect(document.documentElement.lang).toBe("en");
    expect(save).not.toHaveBeenCalled();
  });

  it("sends a language picked here when it is picked, even if its catalog never comes", async () => {
    const english = { chatTranslationEnabled: false, chatTranslationTargetLanguage: "en" };
    world.ports = {
      account: {
        loadClientSettings: vi.fn(async () => ({ ...ACCOUNT, language: english })),
        saveClientSettings: save,
      },
      voiceSettings: new VoiceSettingsStore(null),
    };
    vi.doMock("@/i18n/messages/pl", () => {
      throw new Error("Failed to fetch dynamically imported module");
    });
    const { pickPolish } = await signInOnEnglishDevice();
    await afterSaveWindow();
    expect(save).not.toHaveBeenCalled();

    await act(async () => pickPolish());
    await afterSaveWindow();

    // Otherwise the account's older English would undo the pick on the next load.
    expect(document.documentElement.lang).toBe("en");
    expect(save).toHaveBeenCalledExactlyOnceWith("language", { ...english, locale: "pl" });
  });
});
