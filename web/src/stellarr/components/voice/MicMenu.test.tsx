import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { voiceSettings } from "@/components/account/panes/voiceTestFixtures";
import type { SuppressorDownload, VoiceSettings } from "@/ports";

import { MicMenu } from "./MicMenu";

interface Setup {
  settings?: Partial<VoiceSettings>;
  download?: SuppressorDownload | null;
}

function setup({ settings = {}, download = null }: Setup = {}) {
  const onChangeSettings = vi.fn();
  function Harness({ value }: { value: SuppressorDownload | null }) {
    const [open, setOpen] = useState(false);
    return (
      <MicMenu
        open={open}
        onOpenChange={setOpen}
        settings={{ ...voiceSettings, ...settings }}
        download={value}
        onChangeSettings={onChangeSettings}
        onMoreSettings={vi.fn()}
        caretClassName="vc-caret"
      />
    );
  }
  const view = render(<Harness value={download} />);
  const caret = () => screen.getByRole("button", { name: "Jakość i ustawienia mikrofonu" });
  const menu = () => within(document.querySelector<HTMLElement>(".mic-menu")!);
  return {
    caret,
    menu,
    onChangeSettings,
    rerender: (next: SuppressorDownload | null) => view.rerender(<Harness value={next} />),
  };
}

describe("MicMenu", () => {
  it("changes the audio quality and the noise reduction, automatic first and the modes in the settings' order", async () => {
    const user = userEvent.setup();
    const { caret, menu, onChangeSettings } = setup();

    await user.click(caret());
    const names = (group: string) =>
      within(menu().getByRole("radiogroup", { name: group }))
        .getAllByRole("radio")
        .map((radio) => radio.closest("label")?.querySelector(".qm-label")?.textContent);
    expect(names("Jakość dźwięku")).toEqual([
      "Automatycznie",
      "Oszczędny",
      "Zrównoważony",
      "Wyrazisty",
      "Premium",
      "Studio",
    ]);
    expect(names("Redukcja szumu")).toEqual([
      "Przeglądarki",
      "RNNoise",
      "GTCRN",
      "Speex",
      "Wyłączona",
    ]);

    await user.click(menu().getByRole("radio", { name: /Automatycznie/ }));
    expect(onChangeSettings).toHaveBeenLastCalledWith({ opusProfile: "auto" });
    await user.click(menu().getByRole("radio", { name: /Studio/ }));
    expect(onChangeSettings).toHaveBeenLastCalledWith({ opusProfile: "studio" });
    await user.click(menu().getByRole("radio", { name: "Speex" }));
    expect(onChangeSettings).toHaveBeenLastCalledWith({ noiseSuppression: "speex" });
    await user.click(menu().getByRole("radio", { name: "Wyłączona" }));
    expect(onChangeSettings).toHaveBeenLastCalledWith({ noiseSuppression: "off" });
  });

  it("names both choices in its head and explains each one under its group", async () => {
    const user = userEvent.setup();
    const { caret, menu } = setup({ settings: { opusProfile: "auto", noiseSuppression: "browser" } });

    await user.click(caret());

    expect(menu().getByText("Automatycznie, redukcja przeglądarki")).toBeVisible();
    expect(menu().getByText("do 128 kb/s")).toBeVisible();
    expect(menu().getByText("Dobiera się do łącza, od 128 kb/s w dół.")).toBeVisible();
    expect(menu().getByText("Wbudowana redukcja szumu. Zawsze dostępna.")).toBeVisible();
  });

  it("follows the chosen suppressor's download on its arrow and on its row", async () => {
    const user = userEvent.setup();
    const { caret, menu, rerender } = setup({
      settings: { noiseSuppression: "speex" },
      download: { kind: "speex", percent: 42 },
    });

    expect(caret()).toHaveAttribute("aria-busy", "true");
    expect(caret()).toHaveAttribute("title", "Speex: pobrano 42%");
    expect(caret().querySelector(".progress-ring")).not.toBeNull();
    await user.click(caret());
    expect(menu().getByText("Premium, Speex: pobrano 42%")).toBeVisible();
    const speex = menu().getByRole("radio", { name: /Speex/ });
    expect(speex).toHaveAttribute("aria-busy", "true");
    expect(speex.closest("label")).toHaveTextContent("42%");

    // The total unknown: still busy, in words without a number.
    rerender({ kind: "speex", percent: null });
    expect(caret()).toHaveAttribute("title", "Speex: trwa pobieranie");
    expect(caret()).toHaveAttribute("aria-busy", "true");

    // Arrived: nothing on its way any more, and Speex keeps its own hint.
    rerender(null);
    expect(caret()).not.toHaveAttribute("aria-busy");
    expect(caret().querySelector(".progress-ring")).toBeNull();
    expect(menu().getByRole("radio", { name: /Speex/ })).not.toHaveAttribute("aria-busy");
    expect(menu().getByText("Premium, Speex")).toBeVisible();
    expect(menu().getByText("Klasyczny filtr DSP. Najlżejszy i najsłabszy.")).toBeVisible();
  });

  it("leaves the download of a mode no longer chosen alone", () => {
    const { caret } = setup({
      settings: { noiseSuppression: "speex" },
      download: { kind: "gtcrn", percent: 90 },
    });

    expect(caret()).not.toHaveAttribute("aria-busy");
    expect(caret().querySelector(".progress-ring")).toBeNull();
  });
});
