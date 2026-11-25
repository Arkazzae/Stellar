import { fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { renderInConfirm as render } from "@/test/confirm";
import { DEFAULT_THEME, FONTS, normaliseTheme, type ThemePreference } from "@/design/themes";

import { AppearancePane, type AppearancePaneProps } from "./AppearancePane";

function props(overrides: Partial<AppearancePaneProps> = {}): AppearancePaneProps {
  return {
    theme: DEFAULT_THEME,
    resolvedScheme: "dark",
    onChange: vi.fn(),
    onReset: vi.fn(),
    ...overrides,
  };
}

describe("AppearancePane controls", () => {
  it("hides wallpaper when Midnight is picked and enables reset with only wallpaper active", async () => {
    const user = userEvent.setup();
    const p = props({ theme: { ...DEFAULT_THEME, wallpaper: { shade: 20, blur: 5 } } });
    render(<AppearancePane {...p} />);

    expect(screen.getByRole("button", { name: "Przywróć domyślne" })).toBeEnabled();
    await user.click(screen.getByRole("radio", { name: "Midnight" }));
    expect(p.onChange).toHaveBeenLastCalledWith({
      palette: "midnight",
      accent: DEFAULT_THEME.accent,
      wallpaper: null,
    });
  });

  it("validates pasted CSS, applies recognised tokens and reopens the saved source", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn<(patch: Partial<ThemePreference>) => void>();
    const p = props({ onChange });
    const { rerender } = render(<AppearancePane {...p} />);
    const open = screen.getByRole("button", { name: "Edytuj CSS" });
    await user.click(open);
    const dialog = screen.getByRole("dialog", { name: "Własny motyw CSS" });
    const input = within(dialog).getByRole("textbox", { name: "CSS motywu" });
    const save = within(dialog).getByRole("button", { name: "Zapisz" });
    fireEvent.change(input, { target: { value: ":root { --palette-accent: url(other.css); }" } });
    await user.click(save);
    expect(within(dialog).getByRole("alert")).toHaveTextContent("--palette-accent");
    expect(onChange).not.toHaveBeenCalled();

    const css = "/* Lagoon */\n:root { --palette-accent: #4ba; --palette-background-primary: #123a; }";
    fireEvent.change(input, { target: { value: css } });
    await user.click(save);
    const saved = normaliseTheme({ ...p.theme, ...onChange.mock.lastCall?.[0] });
    expect(saved.palette).toBe("custom");
    expect(saved.accent).toBe("#44bbaa");
    expect(saved.custom?.colors.backgrounds.primary).toBe("#112233aa");
    rerender(<AppearancePane {...p} theme={saved} />);
    await user.click(open);
    expect(screen.getByRole("textbox", { name: "CSS motywu" })).toHaveValue(css);
  });

  it("reports brightness, contrast, face and text size as separate patches", async () => {
    const user = userEvent.setup();
    const p = props();
    render(<AppearancePane {...p} />);

    await user.click(within(screen.getByRole("radiogroup", { name: "Jasność" })).getByRole("radio", { name: /Jasny/ }));
    expect(p.onChange).toHaveBeenLastCalledWith({ scheme: "light" });

    await user.click(screen.getByRole("switch", { name: "Wysoki kontrast" }));
    expect(p.onChange).toHaveBeenLastCalledWith({ contrast: "high" });

    const face = screen.getByRole("combobox", { name: /Krój pisma/ });
    expect(face).toHaveValue("outfit");
    expect(within(face).getAllByRole("option")).toHaveLength(FONTS.length);
    expect(within(face).getByRole("option", { name: "OpenDyslexic" })).toHaveValue("opendyslexic");
    await user.selectOptions(face, "Figtree");
    expect(p.onChange).toHaveBeenLastCalledWith({ font: "figtree" });

    await user.click(within(screen.getByRole("radiogroup", { name: "Rozmiar tekstu" })).getByRole("radio", { name: /Większy/ }));
    expect(p.onChange).toHaveBeenLastCalledWith({ size: "lg" });
  });

  it("marks the theme's own colour while it is in use, and none of the swatches otherwise", () => {
    const { rerender } = render(<AppearancePane {...props()} />);
    const accents = () => screen.getByRole("radiogroup", { name: "Kolor akcentu" });
    expect(within(accents()).getByRole("radio", { name: "Kolor motywu" })).toBeChecked();

    rerender(<AppearancePane {...props({ theme: { ...DEFAULT_THEME, accent: "#ff3366" } })} />);
    expect(within(accents()).queryByRole("radio", { checked: true })).toBeNull();
  });

  it("shows the personal accent in the picker, not the preset's", () => {
    render(<AppearancePane {...props({ theme: { ...DEFAULT_THEME, accent: "#ff3366" } })} />);

    expect(screen.getByLabelText("Własny kolor")).toHaveValue("#ff3366");
  });

  it("does not offer the theme's own colour twice among the swatches", () => {
    render(<AppearancePane {...props()} />);

    // Midnight's accent is the Stellarr indigo, so the indigo swatch stands down.
    const accents = screen.getByRole("radiogroup", { name: "Kolor akcentu" });
    expect(within(accents).queryByRole("radio", { name: "Indygo" })).toBeNull();
  });

  it("moves through the swatches with the arrow keys, choosing as it goes", async () => {
    const user = userEvent.setup();
    const p = props();
    render(<AppearancePane {...p} />);

    within(screen.getByRole("radiogroup", { name: "Kolor akcentu" }))
      .getByRole("radio", { name: "Kolor motywu" })
      .focus();
    await user.keyboard("{ArrowRight}");
    expect(p.onChange).toHaveBeenLastCalledWith({ accent: "#9146ff" });
    expect(screen.getByRole("radio", { name: "Fiolet" })).toHaveFocus();
  });

  it("sets reduced motion with contrast and text size, and resets it with them", async () => {
    const user = userEvent.setup();
    const onMotion = vi.fn();
    const p = props({ motion: { reducedMotion: true, onChange: onMotion } });
    render(<AppearancePane {...p} />);

    await user.click(screen.getByRole("switch", { name: "Ogranicz animacje" }));
    expect(onMotion).toHaveBeenLastCalledWith(false);

    // The theme is all default, but motion is not, so there is something to restore.
    onMotion.mockClear();
    await user.click(screen.getByRole("button", { name: "Przywróć domyślne" }));
    expect(p.onReset).toHaveBeenCalledOnce();
    expect(onMotion).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("leaves reduced motion alone on reset when it is already off", async () => {
    const user = userEvent.setup();
    const onMotion = vi.fn();
    const p = props({
      theme: { ...DEFAULT_THEME, size: "lg" },
      motion: { reducedMotion: false, onChange: onMotion },
    });
    render(<AppearancePane {...p} />);

    await user.click(screen.getByRole("button", { name: "Przywróć domyślne" }));
    expect(p.onReset).toHaveBeenCalledOnce();
    expect(onMotion).not.toHaveBeenCalled();
  });
});
