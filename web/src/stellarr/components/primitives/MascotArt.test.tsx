import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Sticker } from "./MascotArt";
import styles from "./MascotArt.module.css";

/** The class the image takes once its file has loaded. */
const shown = String(styles.shown);

describe("Sticker", () => {
  it("reads the exported file at any size and shows it only once it has loaded", () => {
    const { container, rerender } = render(<Sticker name="02-first-message" size="column" />);
    const first = container.querySelector("img") as HTMLImageElement;

    expect(first).toHaveAttribute("src", "/mascot/02-first-message-104.webp");
    expect(first).not.toHaveClass(shown);
    fireEvent.load(first);
    expect(first).toHaveClass(shown);

    // Another moment starts out of sight again, and a file that fails stays there.
    rerender(<Sticker name="33-finding-message" size="main" />);
    const second = container.querySelector("img") as HTMLImageElement;
    expect(second).toHaveAttribute("src", "/mascot/33-finding-message-104.webp");
    fireEvent.error(second);
    expect(second).not.toHaveClass(shown);
  });
});
