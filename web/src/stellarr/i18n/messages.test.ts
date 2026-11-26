import { describe, expect, it } from "vitest";

import en from "../../../.locales/web/en.json";
import pl from "../../../.locales/web/pl.json";
import { translateMessage } from ".";

describe("interface message catalogs", () => {
  it("keeps every locale on the same set of keys", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(pl).sort());
  });

  it("interpolates named values without evaluating message content", () => {
    expect(translateMessage("en", "language.model.preparing", { size: 22 }))
      .toBe("Preparing a 22 MB local model…");
  });
});

