import { describe, expect, it } from "vitest";

import { reservedImageStyle } from "./imageBox";

describe("reservedImageStyle", () => {
  it("hands the stylesheet the measured width and the ratio", () => {
    expect(reservedImageStyle(1200, 800)).toEqual({
      "--att-image-w": "1200px",
      "--att-image-ratio": "1200 / 800",
    });
    expect(reservedImageStyle(600, 1200)).toEqual({
      "--att-image-w": "600px",
      "--att-image-ratio": "600 / 1200",
    });
  });

  it("reserves nothing without both dimensions", () => {
    expect(reservedImageStyle()).toBeUndefined();
    expect(reservedImageStyle(1200)).toBeUndefined();
    expect(reservedImageStyle(undefined, 800)).toBeUndefined();
  });

  it("reserves nothing for a size no image has", () => {
    expect(reservedImageStyle(0, 800)).toBeUndefined();
    expect(reservedImageStyle(1200, -1)).toBeUndefined();
    expect(reservedImageStyle(Number.NaN, 800)).toBeUndefined();
    expect(reservedImageStyle(1200, Number.POSITIVE_INFINITY)).toBeUndefined();
  });
});
