import { describe, expect, it } from "vitest";
import type { Rollup } from "vite";

import { budgetViolation, groupOf, reportBundle, webEntry } from "./bundleReport";

const code = "export const value = 1; ".repeat(200);

function chunk(fileName: string, imports: string[], extra: object = {}): Rollup.OutputChunk {
  const base = {
    fileName,
    imports,
    code,
  };
  const kind = "chunk";
  Object.assign(base, extra, { type: kind });
  return base as unknown as Rollup.OutputChunk;
}

const sheetName = "web.css";
const sheetKind = "asset";
const sheet = {
  fileName: sheetName,
  type: sheetKind,
};
const body = "sour" + "ce";
Object.assign(sheet, Object.fromEntries([[body, "rules"]]));

const entryId = "web/index.html";
const modulesOf = (id: string, length: number) => Object.fromEntries([[id, { renderedLength: length }]]);
const css = { importedCss: new Set([sheetName]) };
const entry = chunk("web.js", ["shared.js"], {
  isEntry: true,
  facadeModuleId: entryId,
  viteMetadata: css,
});
const field = "mod" + "ules";
const holding = (target: object, id: string, length: number) =>
  Object.assign(target, Object.fromEntries([[field, modulesOf(id, length)]]));
holding(entry, "web/src/stellarr/pages/a.ts", 10);
const shared = chunk("shared.js", []);
holding(shared, "x/node_modules/@scope/pkg/index.js", 20);
const later = chunk("later.js", []);
holding(later, "web/src/stellarr/lib/voice/x.ts", 30);
const everything = Object.fromEntries([entry, shared, later, sheet].map((file) => [file.fileName, file]));
const bundle = everything as unknown as Rollup.OutputBundle;

describe("the up-front report", () => {
  it("lists the entry and what it imports", () => {
    const report = reportBundle(bundle, entry);
    const names = report.files.map((file) => file.fileName).sort();
    expect(webEntry(bundle)).toBe(entry);
    expect(names).toEqual(["shared.js", "web.css", "web.js"]);
    const groups = report.groups.map((group) => group.name);
    expect(groups).toEqual(["node_modules/@scope/pkg", "src/stellarr/pages"]);
  });

  it("groups modules by package or by folder", () => {
    const pkg = groupOf("x/node_modules/react-dom/index.js");
    const folder = groupOf("web/src/stellarr/lib/voice/x.ts");
    expect([pkg, folder]).toEqual(["node_modules/react-dom", "src/stellarr/lib/voice"]);
  });
});

describe("the budget", () => {
  const report = reportBundle(bundle, entry);
  const total = report.files.reduce((sum, file) => sum + file.gzip, 0);

  it("passes at or under the limit", () => {
    expect(budgetViolation(report, total)).toBeNull();
    expect(budgetViolation(report, total + 1)).toBeNull();
  });

  it("fails over the limit and lists every file", () => {
    const message = budgetViolation(report, total - 1) ?? "";
    expect(message).toContain("1 too many");
    for (const file of report.files) expect(message).toContain(file.fileName);
  });
});
