import { fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { encodePlaceholder, placeholderToBase64 } from "@/lib/media/placeholder";

import { WallpaperLayer } from "./WallpaperLayer";

const ASSET = "/assets/3f2b8c1e-0d4a-4e6b-9c71-5a8e2f9d1b20";
const root = document.documentElement;

/** A 2×2 picture in the synced few bytes. */
const PLACEHOLDER = placeholderToBase64(
  encodePlaceholder({ width: 2, height: 2, data: new Uint8ClampedArray(Array.from({ length: 16 }, (_, i) => (i % 4 === 3 ? 255 : 40 * i))) }),
);

afterEach(() => {
  delete root.dataset["wallpaper"];
});

const layer = () => document.body.querySelector<HTMLElement>(".wallpaper");

describe("WallpaperLayer", () => {
  it("paints the stand-in, the picture once it loads, and the shading, behind the app", () => {
    const { unmount } = render(
      <WallpaperLayer settings={{ shade: 70, blur: 6, placeholder: PLACEHOLDER }} url={ASSET} known />,
    );

    expect(root.dataset["wallpaper"]).toBe("on");
    expect(layer()?.parentElement).toBe(document.body);
    const [standIn, picture] = layer()?.querySelectorAll("img") ?? [];
    expect(standIn?.getAttribute("src")).toMatch(/^data:image\/png;base64,/);
    expect(picture?.getAttribute("src")).toBe(ASSET);
    expect(picture?.style.filter).toBe("blur(6px)");
    expect(picture).not.toHaveClass("on");
    fireEvent.load(picture!);
    expect(picture).toHaveClass("on");
    expect(layer()?.querySelector<HTMLElement>(".wallpaper-shade")?.style.opacity).toBe("0.7");

    // Signing out unmounts the app, and the sign-in page is never see-through.
    unmount();
    expect(root.dataset["wallpaper"]).toBeUndefined();
  });

  it("keeps the see-through layout while the account loads and drops it for an account without a picture", () => {
    const { rerender } = render(<WallpaperLayer settings={{ shade: 80, blur: 0 }} url={undefined} known={false} />);
    expect(root.dataset["wallpaper"]).toBe("on");

    rerender(<WallpaperLayer settings={{ shade: 80, blur: 0 }} url={undefined} known />);
    expect(root.dataset["wallpaper"]).toBeUndefined();
    expect(layer()).toBeNull();
  });

  it("shows only a picture on our own host", () => {
    render(<WallpaperLayer settings={{ shade: 80, blur: 0 }} url="https://example.com/x.jpg" known />);
    expect(layer()).toBeNull();
    expect(root.dataset["wallpaper"]).toBeUndefined();
  });
});
