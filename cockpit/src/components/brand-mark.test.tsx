import { readFileSync } from "node:fs";
import { join } from "node:path";

import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { BrandMark, MARK_PATH } from "./brand-mark";

describe("BrandMark", () => {
  it("draws the outline of the brand's one-colour mark", () => {
    const source = readFileSync(join(process.cwd(), "..", "branding", "logo", "mono.svg"), "utf8");

    expect(/ d="([^"]+)"/.exec(source)?.[1]).toBe(MARK_PATH);
  });

  it("fills it with the colour of the text around it and hides it from assistive tech", () => {
    const { container } = render(<BrandMark className="size-6" />);

    const mark = container.firstElementChild;
    expect(mark?.getAttribute("aria-hidden")).toBe("true");
    expect(mark?.className).toBe("size-6");
    expect(mark?.querySelector("path")?.getAttribute("fill")).toBe("currentColor");
  });
});
