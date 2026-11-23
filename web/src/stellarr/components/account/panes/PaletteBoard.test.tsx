import { fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { forkPreset, withColour } from "@/design/customTheme";
import { DEFAULT_THEME, getPalette, type ThemePreference } from "@/design/themes";
import { answerConfirm, renderInConfirm } from "@/test/confirm";

import { AppearancePane, type AppearancePaneProps } from "./AppearancePane";

function props(theme: ThemePreference): AppearancePaneProps {
  return { theme, resolvedScheme: "dark", onChange: vi.fn(), onReset: vi.fn() };
}

/** The person's own palette: Harbor with a red primary background. */
const OWN = { ...withColour(forkPreset("harbor"), "backgrounds", "primary", "#330000"), name: "Port" };

function board() {
  return screen.getByText("Pełna paleta").closest("details") as HTMLElement;
}

describe("full palette", () => {
  it("shows every colour of the palette in use, grouped, with its code", () => {
    renderInConfirm(<AppearancePane {...props({ ...DEFAULT_THEME, palette: "ink" })} />);

    const backgrounds = screen.getByRole("region", { name: "Tła" });
    expect(within(backgrounds).getAllByRole("listitem")).toHaveLength(8);
    expect(within(screen.getByRole("region", { name: "Obramowania" })).getByLabelText("Subtelne, kod koloru")).toHaveValue("#f0f6fc14");
    expect(within(screen.getByRole("region", { name: "Stany" })).getAllByRole("listitem")).toHaveLength(4);
  });

  it("makes a changed preset the person's own palette and selects it", () => {
    const p = props({ ...DEFAULT_THEME, palette: "ink", accent: "#ff3366" });
    renderInConfirm(<AppearancePane {...p} />);

    fireEvent.change(screen.getByLabelText("Wybierz kolor: Główne"), { target: { value: "#203040" } });

    const patch = vi.mocked(p.onChange).mock.lastCall?.[0];
    expect(patch?.palette).toBe("custom");
    // The accent the person chose is not theirs to lose by changing a background.
    expect(patch).not.toHaveProperty("accent");
    expect(patch?.custom).toMatchObject({ name: "Mój Ink", basedOn: "ink" });
    expect(patch?.custom?.colors.backgrounds.primary).toBe("#203040");
    expect(patch?.custom?.colors.text).toEqual(getPalette("ink").colors.text);
  });

  it("keeps a border's opacity through the picker and takes a new one as a typed code", async () => {
    const user = userEvent.setup();
    const p = props({ ...DEFAULT_THEME, palette: "custom", custom: OWN });
    renderInConfirm(<AppearancePane {...p} />);

    fireEvent.change(screen.getByLabelText("Wybierz kolor: Subtelne"), { target: { value: "#00ff00" } });
    expect(vi.mocked(p.onChange).mock.lastCall?.[0].custom?.colors.borders.subtle).toBe("#00ff0014");

    const code = screen.getByLabelText("Mocne, kod koloru");
    await user.clear(code);
    await user.type(code, "66c0f480{Enter}");
    expect(vi.mocked(p.onChange).mock.lastCall?.[0].custom?.colors.borders.strong).toBe("#66c0f480");
  });

  it("refuses a code that is not a hex colour and shows the colour it had", async () => {
    const user = userEvent.setup();
    const p = props({ ...DEFAULT_THEME, palette: "custom", custom: OWN });
    renderInConfirm(<AppearancePane {...p} />);

    const code = screen.getByLabelText("Główne, kod koloru");
    await user.clear(code);
    await user.type(code, "url(x)");
    expect(code).toHaveAttribute("aria-invalid", "true");
    await user.tab();

    expect(p.onChange).not.toHaveBeenCalled();
    expect(code).toHaveValue("#330000");
  });

  it("offers the person's own palette as a tile painted by its own colours", async () => {
    const user = userEvent.setup();
    const p = props({ ...DEFAULT_THEME, palette: "slate", custom: OWN });
    renderInConfirm(<AppearancePane {...p} />);

    const tile = screen.getByRole("radio", { name: /Port/ });
    expect(tile).toHaveAttribute("data-theme", "custom");
    expect(tile.style.getPropertyValue("--palette-background-primary")).toBe("#330000");
    expect(tile.style.getPropertyValue("--h-neutral")).toBe(String(OWN.neutral.hue));

    await user.click(tile);
    expect(p.onChange).toHaveBeenLastCalledWith({ palette: "custom", accent: OWN.colors.accent });
  });

  it("asks before a changed preset replaces the person's own palette", async () => {
    const user = userEvent.setup();
    const p = props({ ...DEFAULT_THEME, palette: "ink", custom: OWN });
    renderInConfirm(<AppearancePane {...p} />);

    fireEvent.change(screen.getByLabelText("Wybierz kolor: Główne"), { target: { value: "#111111" } });
    fireEvent.change(screen.getByLabelText("Wybierz kolor: Główne"), { target: { value: "#222222" } });
    expect(p.onChange).not.toHaveBeenCalled();

    await answerConfirm(user, "Zastąp");
    expect(p.onChange).toHaveBeenCalledTimes(1);
    expect(vi.mocked(p.onChange).mock.lastCall?.[0].custom?.colors.backgrounds.primary).toBe("#222222");
  });

  it("removes the person's own palette only once confirmed, back to the preset it came from", async () => {
    const user = userEvent.setup();
    const p = props({ ...DEFAULT_THEME, palette: "custom", custom: OWN });
    renderInConfirm(<AppearancePane {...p} />);

    await user.click(within(board()).getByRole("button", { name: "Usuń własny motyw" }));
    await answerConfirm(user, "Anuluj");
    expect(p.onChange).not.toHaveBeenCalled();

    await user.click(within(board()).getByRole("button", { name: "Usuń własny motyw" }));
    await answerConfirm(user, "Usuń");
    expect(p.onChange).toHaveBeenLastCalledWith({ palette: "harbor", accent: "#1a9fff", custom: null });
  });

  it("renames the person's own palette when the field is left", async () => {
    const user = userEvent.setup();
    const p = props({ ...DEFAULT_THEME, palette: "custom", custom: OWN });
    renderInConfirm(<AppearancePane {...p} />);

    const name = within(board()).getByLabelText("Nazwa motywu");
    await user.clear(name);
    await user.type(name, "Nocny port ");
    expect(p.onChange).not.toHaveBeenCalled();
    await user.tab();

    expect(vi.mocked(p.onChange).mock.lastCall?.[0].custom?.name).toBe("Nocny port");
  });
});
