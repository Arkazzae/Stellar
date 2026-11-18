import { gzipSync } from "node:zlib";

import type { Plugin, Rollup } from "vite";

import { staticClosure, WEB_DOCUMENT } from "./documents";

/*
 * What the web client's document loads before it can paint, measured from the
 * bundle: the files of its entry's static import closure and the stylesheets
 * those chunks name, and which modules the chunks are made of. The build
 * prints it, so that a change that puts weight on the sign-in page shows in
 * the same run that caused it.
 */

/** One file of the document's up-front set. */
export interface UpFrontFile {
  fileName: string;
  /** Bytes as written. */
  raw: number;
  /** Bytes after gzip, which is what the network carries. */
  gzip: number;
}

/** A group of modules, as the report names it, and what they add to the up-front chunks. */
export interface ModuleGroup {
  name: string;
  /** Rendered length of the group's modules in the chunks, before compression. */
  rendered: number;
}

export interface BundleReport {
  files: UpFrontFile[];
  groups: ModuleGroup[];
}

const gzipSize = (content: string | Uint8Array): number => gzipSync(content).length;

const isWebEntry = (output: Rollup.OutputBundle[string]): output is Rollup.OutputChunk =>
  output.type === "chunk" &&
  output.isEntry &&
  output.facadeModuleId?.endsWith(`/${WEB_DOCUMENT}`) === true &&
  !output.facadeModuleId.endsWith("/cockpit/index.html");

/** The chunk the web client's document starts, or undefined in a build without it. */
export function webEntry(bundle: Rollup.OutputBundle): Rollup.OutputChunk | undefined {
  return Object.values(bundle).find(isWebEntry);
}

const SOURCE_ROOT = "/src/stellarr/";
const PACKAGES = "/node_modules/";

/**
 * The name a module is counted under: its package for `node_modules`, its
 * directory under `src/stellarr` for the client's own code (two levels for
 * `lib`, which holds unrelated subsystems), the workspace package's name for
 * the packages the client shares.
 */
export function groupOf(id: string): string {
  const path = id.split("?")[0]?.replaceAll("\\", "/") ?? id;
  const modules = path.lastIndexOf(PACKAGES);
  if (modules >= 0) {
    const parts = path.slice(modules + PACKAGES.length).split("/");
    const name = parts[0]?.startsWith("@") === true ? `${parts[0]}/${parts[1] ?? ""}` : (parts[0] ?? path);
    return `node_modules/${name}`;
  }
  const source = path.indexOf(SOURCE_ROOT);
  if (source >= 0) {
    const parts = path.slice(source + SOURCE_ROOT.length).split("/");
    if (parts.length === 1) return "src/stellarr (files)";
    const depth = parts[0] === "lib" && parts.length > 2 ? 2 : 1;
    return `src/stellarr/${parts.slice(0, depth).join("/")}`;
  }
  const packages = path.indexOf("/packages/");
  if (packages >= 0) return `packages/${path.slice(packages + "/packages/".length).split("/")[0] ?? ""}`;
  return "other";
}

/** Files of the entry's static closure with their sizes, and what its chunks are made of. */
export function reportBundle(bundle: Rollup.OutputBundle, entry: Rollup.OutputChunk): BundleReport {
  const chunks = new Map(
    Object.values(bundle)
      .filter((output): output is Rollup.OutputChunk => output.type === "chunk")
      .map((chunk) => [chunk.fileName, chunk]),
  );
  const files: UpFrontFile[] = [];
  const totals = new Map<string, number>();
  const stylesheets = new Set<string>();

  for (const fileName of staticClosure(chunks, entry.fileName)) {
    const chunk = chunks.get(fileName);
    if (chunk === undefined) continue;
    files.push({ fileName, raw: Buffer.byteLength(chunk.code), gzip: gzipSize(chunk.code) });
    for (const [id, module] of Object.entries(chunk.modules)) {
      const group = groupOf(id);
      totals.set(group, (totals.get(group) ?? 0) + module.renderedLength);
    }
    for (const css of chunk.viteMetadata?.importedCss ?? []) stylesheets.add(css);
  }
  for (const fileName of stylesheets) {
    const asset = bundle[fileName];
    if (asset?.type !== "asset") continue;
    files.push({ fileName, raw: Buffer.byteLength(asset.source), gzip: gzipSize(asset.source) });
  }
  const groups = [...totals]
    .map(([name, rendered]) => ({ name, rendered }))
    .sort((a, b) => b.rendered - a.rendered || a.name.localeCompare(b.name));
  return { files, groups };
}

const kilobytes = (bytes: number) => `${(bytes / 1000).toFixed(1)} kB`;

/** The report as two tables. */
export function formatReport(report: BundleReport, groupLimit = 25): string {
  const nameWidth = Math.max(...report.files.map((file) => file.fileName.length), "total".length);
  const rows = report.files.map(
    (file) => `  ${file.fileName.padEnd(nameWidth)}  ${kilobytes(file.raw).padStart(10)}  ${kilobytes(file.gzip).padStart(10)}`,
  );
  const raw = report.files.reduce((sum, file) => sum + file.raw, 0);
  const gzip = report.files.reduce((sum, file) => sum + file.gzip, 0);
  const renderedTotal = report.groups.reduce((sum, group) => sum + group.rendered, 0);
  const shown = report.groups.slice(0, groupLimit);
  const groupWidth = Math.max(...shown.map((group) => group.name.length), 0);
  const modules = shown.map(
    (group) =>
      `  ${group.name.padEnd(groupWidth)}  ${kilobytes(group.rendered).padStart(10)}  ${`${((group.rendered / renderedTotal) * 100).toFixed(1)}%`.padStart(6)}`,
  );
  return [
    `Files the web document loads up front (${WEB_DOCUMENT})`,
    `  ${"file".padEnd(nameWidth)}  ${"raw".padStart(10)}  ${"gzip".padStart(10)}`,
    ...rows,
    `  ${"total".padEnd(nameWidth)}  ${kilobytes(raw).padStart(10)}  ${kilobytes(gzip).padStart(10)}`,
    "",
    `What the script chunks are made of, by rendered length (top ${shown.length} of ${report.groups.length})`,
    ...modules,
  ].join("\n");
}

/** Prints the report once the bundle is complete. */
export function bundleReport(): Plugin {
  return {
    name: "stellarr-bundle-report",
    apply: "build",
    generateBundle(_options, bundle) {
      const entry = webEntry(bundle);
      if (entry === undefined) return;
      console.log(`\n${formatReport(reportBundle(bundle, entry))}\n`);
    },
  };
}
