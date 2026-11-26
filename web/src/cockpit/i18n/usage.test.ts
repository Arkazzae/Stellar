import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import catalogs from "../../../.locales/cockpit.json";

/** Cockpit's code, without its catalogs and tests. */
const cockpit = join(process.cwd(), "src", "cockpit");
const sources = readdirSync(cockpit, { recursive: true, encoding: "utf8" })
  .filter((file) => /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) && !file.startsWith("i18n"))
  .map((file) => readFileSync(join(cockpit, file), "utf8"));

describe("the catalog", () => {
  it("has no key that the interface never asks for", () => {
    const code = sources.join("\n");
    const unused = Object.keys(catalogs.en).filter((key) => !code.includes(`"${key}"`));

    expect(unused).toEqual([]);
  });
});
