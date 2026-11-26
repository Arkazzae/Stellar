import { render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_THEME, THEME_STORAGE_KEY } from "@/design/themes";
import { ThemeProvider } from "@/state/ThemeProvider";

import { AppAppearance } from "./AppAppearance";

// The account as it is before it has loaded: who has which wallpaper is the layer's own test.
vi.mock("@/state/hooks", async (original) => ({
  ...(await original<object>()),
  useAccount: () => ({ account: { id: "" } }),
}));

const root = document.documentElement;

afterEach(() => {
  localStorage.clear();
  delete root.dataset["font"];
  delete root.dataset["wallpaper"];
});

describe("AppAppearance", () => {
  it("ignores a legacy font and shows the saved wallpaper only in the signed-in app", () => {
    localStorage.setItem(
      THEME_STORAGE_KEY,
      JSON.stringify({ ...DEFAULT_THEME, font: "figtree", wallpaper: { shade: 80, blur: 0 } }),
    );
    // The provider alone, as on the sign-in pages, even with the app's
    // stylesheets fetched ahead: the brand's face, nothing see-through.
    const signIn = render(<ThemeProvider>{null}</ThemeProvider>);
    expect(root.dataset["font"]).toBeUndefined();
    expect(root.dataset["wallpaper"]).toBeUndefined();
    signIn.unmount();

    const app = render(
      <ThemeProvider>
        <AppAppearance />
      </ThemeProvider>,
    );
    expect(root.dataset["font"]).toBeUndefined();
    expect(root.dataset["wallpaper"]).toBe("on");

    // Signing out leaves the sign-in page as it was.
    app.unmount();
    expect(root.dataset["font"]).toBeUndefined();
    expect(root.dataset["wallpaper"]).toBeUndefined();
  });
});
