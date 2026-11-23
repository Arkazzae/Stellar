import { fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_THEME, WALLPAPER_SHADE_DEFAULT, type ThemePreference } from "@/design/themes";
import { prepareWallpaper } from "@/lib/wallpaper/prepareWallpaper";
import { answerConfirm, renderInConfirm } from "@/test/confirm";

import { WallpaperSection } from "./WallpaperSection";
import type { WallpaperAccess } from "./wallpaperUpload";

// The browser's decoder and canvas are not jsdom's; what they make is stood in for.
vi.mock("@/lib/wallpaper/prepareWallpaper", () => ({ prepareWallpaper: vi.fn() }));

const ASSET = "/assets/3f2b8c1e-0d4a-4e6b-9c71-5a8e2f9d1b20";
const PREPARED = new File(["webp"], "wallpaper.webp", { type: "image/webp" });

function access(overrides: Partial<WallpaperAccess> = {}): WallpaperAccess {
  return {
    upload: vi.fn(async () => ({ ok: true as const })),
    remove: vi.fn(async () => undefined),
    read: vi.fn(async () => new Blob()),
    ...overrides,
  };
}

function render(theme: ThemePreference, wallpaper: WallpaperAccess) {
  const onChange = vi.fn();
  const view = renderInConfirm(<WallpaperSection theme={theme} onChange={onChange} wallpaper={wallpaper} />);
  const input = view.container.querySelector<HTMLInputElement>('input[type="file"]')!;
  return { onChange, input };
}

beforeEach(() => {
  vi.mocked(prepareWallpaper).mockResolvedValue({ file: PREPARED, width: 2560, height: 1440, placeholder: "AAAA", firstFrameOnly: false });
});

describe("wallpaper section", () => {
  it("uploads the prepared picture and only then sets the wallpaper, at the default shading", async () => {
    const user = userEvent.setup();
    const wallpaper = access();
    const { onChange, input } = render(DEFAULT_THEME, wallpaper);

    await user.upload(input, new File(["jpeg"], "holiday.jpg", { type: "image/jpeg" }));

    await vi.waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(wallpaper.upload).toHaveBeenCalledWith(PREPARED);
    expect(onChange).toHaveBeenCalledWith({ wallpaper: { shade: WALLPAPER_SHADE_DEFAULT, blur: 0, placeholder: "AAAA" } });
  });

  it("says what the server said when there is no room, and sets nothing", async () => {
    const user = userEvent.setup();
    const wallpaper = access({ upload: vi.fn(async () => ({ ok: false as const, message: "Brak miejsca na serwerze." })) });
    const { onChange, input } = render(DEFAULT_THEME, wallpaper);

    await user.upload(input, new File(["jpeg"], "holiday.jpg", { type: "image/jpeg" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Brak miejsca na serwerze.");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("refuses a vector picture before anything is uploaded", async () => {
    // The picker offers raster pictures, but a file can still be dropped or forced in.
    const user = userEvent.setup({ applyAccept: false });
    vi.mocked(prepareWallpaper).mockResolvedValue({ problem: "vector" });
    const wallpaper = access();
    const { input } = render(DEFAULT_THEME, wallpaper);

    await user.upload(input, new File(["<svg/>"], "logo.svg", { type: "image/svg+xml" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Obraz SVG nie może być tapetą.");
    expect(wallpaper.upload).not.toHaveBeenCalled();
  });

  it("changes the shading of a set wallpaper, and removes it only once confirmed", async () => {
    const user = userEvent.setup();
    const wallpaper = access({ url: ASSET });
    const theme = { ...DEFAULT_THEME, wallpaper: { shade: 80, blur: 0, placeholder: "AAAA" } };
    const { onChange } = render(theme, wallpaper);

    const shade = screen.getByRole("slider", { name: "Przyciemnienie" });
    fireEvent.change(shade, { target: { value: "75" } });
    expect(onChange).toHaveBeenLastCalledWith({ wallpaper: { shade: 75, blur: 0, placeholder: "AAAA" } });

    await user.click(screen.getByRole("button", { name: "Usuń" }));
    await answerConfirm(user, "Usuń");
    expect(wallpaper.remove).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenLastCalledWith({ wallpaper: null });
  });
});
