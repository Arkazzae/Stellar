import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { forkPreset } from "@/design/customTheme";
import { themeFileOf, themeFileText } from "@/design/themeFile";
import { DEFAULT_THEME, type ThemePreference } from "@/design/themes";
import { answerConfirm, renderInConfirm } from "@/test/confirm";

import { ThemeFileRow } from "./ThemeFileRow";

const HARBOR_FILE = themeFileText(themeFileOf({ ...DEFAULT_THEME, palette: "harbor", accent: "#ff3366" }));

function picker(container: HTMLElement) {
  return container.querySelector<HTMLInputElement>('input[type="file"]')!;
}

function render(theme: ThemePreference) {
  const onChange = vi.fn();
  const view = renderInConfirm(<ThemeFileRow theme={theme} onChange={onChange} />);
  return { onChange, input: picker(view.container) };
}

describe("theme file row", () => {
  it("applies a picked file as the person's own palette, in the dark scheme", async () => {
    const user = userEvent.setup();
    const { onChange, input } = render({ ...DEFAULT_THEME, scheme: "light" });

    await user.upload(input, new File([HARBOR_FILE], "Harbor.stellarr-theme.json", { type: "application/json" }));

    await vi.waitFor(() => expect(onChange).toHaveBeenCalledOnce());
    expect(onChange.mock.lastCall?.[0]).toMatchObject({ palette: "custom", accent: "#ff3366", scheme: "dark", custom: { name: "Harbor" } });
  });

  it("asks before a file replaces the person's own palette", async () => {
    const user = userEvent.setup();
    const { onChange, input } = render({ ...DEFAULT_THEME, custom: forkPreset("ink") });

    await user.upload(input, new File([HARBOR_FILE], "Harbor.stellarr-theme.json"));
    await answerConfirm(user, "Anuluj");
    expect(onChange).not.toHaveBeenCalled();

    await user.upload(input, new File([HARBOR_FILE], "Harbor.stellarr-theme.json"));
    await answerConfirm(user, "Zastąp");
    expect(onChange).toHaveBeenCalledOnce();
  });

  it("says why a file was refused and changes nothing", async () => {
    const user = userEvent.setup();
    const { onChange, input } = render(DEFAULT_THEME);

    await user.upload(input, new File(['{"name":"x"}'], "x.json"));

    expect(await screen.findByRole("alert")).toHaveTextContent("To nie jest plik motywu Stellarr.");
    expect(onChange).not.toHaveBeenCalled();
  });
});
