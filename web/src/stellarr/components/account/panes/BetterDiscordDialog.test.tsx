import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_THEME, getPalette } from "@/design/themes";
import type { Conversion } from "@/lib/betterDiscord/convert";
import { convertAddress, convertText, fetchPicture } from "@/lib/betterDiscord/run";
import { renderInConfirm } from "@/test/confirm";

import BetterDiscordDialog from "./BetterDiscordDialog";

// The conversion itself is tested on its own; here it is what the dialog shows and does with one.
vi.mock("@/lib/betterDiscord/run", async (original) => ({
  ...(await original<object>()),
  convertAddress: vi.fn(),
  convertText: vi.fn(),
  fetchPicture: vi.fn(),
}));

const PALETTE = getPalette("ink").colors;

function conversion(overrides: Partial<Conversion> = {}): Conversion {
  return {
    meta: { name: "nocturne", author: "ada" },
    palette: PALETTE,
    accent: "#46aec5",
    roles: [{ group: "backgrounds", role: "primary", from: "--background-base-low" }, { group: "text", role: "muted" }],
    missingCore: [],
    font: { family: "figtree", id: "figtree" },
    styleQueries: "native",
    undecided: 0,
    skipped: [],
    fetched: [],
    ...overrides,
  };
}

async function open(result: Conversion) {
  const user = userEvent.setup();
  vi.mocked(convertText).mockResolvedValue(result);
  const onChange = vi.fn();
  const onClose = vi.fn();
  const upload = vi.fn(async () => ({ ok: true as const }));
  const view = renderInConfirm(
    <BetterDiscordDialog
      open
      onClose={onClose}
      theme={DEFAULT_THEME}
      onChange={onChange}
      wallpaper={{ upload, remove: vi.fn(), read: vi.fn() }}
    />,
  );
  const input = view.baseElement.querySelector<HTMLInputElement>('input[type="file"]')!;
  await user.upload(input, new File([":root{}"], "nocturne.theme.css", { type: "text/css" }));
  await screen.findByText("nocturne");
  return { user, onChange, onClose, upload };
}

describe("BetterDiscord dialog", () => {
  it("shows what came across and applies it as the person's own palette, in the dark scheme", async () => {
    const { user, onChange, onClose } = await open(conversion());

    const taken = screen.getByRole("region", { name: "Przeniesione" });
    expect(within(taken).getByText("Krój pisma Figtree")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Pominięte" })).getByText(/Układ, odstępy/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Zastosuj" }));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ palette: "custom", scheme: "dark", accent: "#46aec5", font: "figtree", custom: expect.objectContaining({ name: "nocturne", author: "ada" }) }),
    );
    expect(onClose).toHaveBeenCalled();
  });

  it("opens a GitHub repository result and converts the chosen theme file", async () => {
    const user = userEvent.setup();
    const repo = "https://github.com/ada/nocturne";
    const file = "https://raw.githubusercontent.com/ada/nocturne/HEAD/themes/Nocturne.theme.css";
    vi.mocked(convertAddress)
      .mockResolvedValueOnce({ groups: [
        { provider: "betterDiscord", choices: [], problem: "unavailable" },
        { provider: "github", choices: [{ label: "ada/nocturne", url: repo }] },
      ] })
      .mockResolvedValueOnce({ groups: [{ provider: "files", choices: [{ label: "themes/Nocturne.theme.css", url: file }] }] })
      .mockResolvedValueOnce({ conversion: conversion(), source: file });

    renderInConfirm(<BetterDiscordDialog open onClose={vi.fn()} theme={DEFAULT_THEME} onChange={vi.fn()} />);
    await user.type(screen.getByRole("textbox", { name: "Nazwa lub link do motywu" }), "nocturne{Enter}");
    await user.click(await screen.findByRole("button", { name: "ada/nocturne" }));
    await user.click(await screen.findByRole("button", { name: "themes/Nocturne.theme.css" }));
    expect(await screen.findByRole("button", { name: "Zastosuj" })).toBeInTheDocument();
    expect(vi.mocked(convertAddress).mock.calls.map(([source]) => source)).toEqual(["nocturne", repo, file]);
  });

  it("offers only what came across when the theme has no palette, and fetches its picture only when asked", async () => {
    const { user, onChange, upload } = await open(
      conversion({
        palette: undefined,
        missingCore: ["backgrounds.primary"],
        font: { family: "gg sans", id: null },
        wallpaper: { url: "https://clearvision.github.io/images/sapphire.jpg", host: "clearvision.github.io", shade: 40 },
      }),
    );

    expect(screen.getByText(/Motyw nie zawiera tła/)).toBeInTheDocument();
    expect(screen.getByText("Krój gg sans, którego nie ma w pakiecie")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Zastosuj to, co przeniesione" }));

    expect(onChange).toHaveBeenCalledWith({ accent: "#46aec5" });
    expect(fetchPicture).not.toHaveBeenCalled();
    expect(upload).not.toHaveBeenCalled();
  });
});
