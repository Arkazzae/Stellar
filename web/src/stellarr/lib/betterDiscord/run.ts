import { imageFromDataAddress } from "@/design/themeFile";

import { convertTheme, type Conversion } from "./convert";
import { browserEvaluator } from "./evaluate";
import {
  SourceError,
  canonicalStylesheetUrl,
  fetchSource,
  findGithubFiles,
  readSource,
  releasedStylesheetUrl,
  searchThemes,
  storeTheme,
  type SourceGroup,
  type ThemeChoice,
} from "./sources";

async function fetchSheet(url: string, signal: AbortSignal, release?: string) {
  const address = canonicalStylesheetUrl(url);
  const response = await fetchSource(address, "css", signal).catch((error: unknown) => {
    if (release !== undefined && error instanceof SourceError && (error.status === 404 || error.status === 410)) {
      const archived = releasedStylesheetUrl(address, release);
      if (archived !== undefined) return fetchSource(archived, "css", signal);
    }
    throw error;
  });
  const text = await response.text();
  if (/^\s*</.test(text)) throw new Error("Expected a stylesheet.");
  return { text, url: response.headers.get("x-theme-source") ?? address };
}

function fileName(url: string): string {
  const name = url.split("/").at(-1) ?? "";
  return name.replace(/(?:\.theme)?\.css$/i, "");
}

export async function convertText(text: string, name?: string): Promise<Conversion> {
  const deadline = AbortSignal.timeout(25_000);
  const conversion = await convertTheme(text, null, { fetchSheet: (url) => fetchSheet(url, deadline), evaluator: browserEvaluator() });
  if (name !== undefined) conversion.meta.name ??= fileName(name);
  return conversion;
}

type AddressResult =
  | { conversion: Conversion; source: string }
  | { groups: SourceGroup[] }
  | { problem: "notHttps" | "invalid" | "notFound" };

async function convertUrl(url: string, choice?: ThemeChoice): Promise<AddressResult> {
  const deadline = AbortSignal.timeout(25_000);
  const sheet = await fetchSheet(url, deadline);
  const conversion = await convertTheme(sheet.text, sheet.url, {
    fetchSheet: (address) => fetchSheet(address, deadline, sheet.url),
    evaluator: browserEvaluator(),
  });
  conversion.meta.name ??= fileName(choice?.label ?? new URL(sheet.url).pathname);
  if (choice?.detail !== undefined) conversion.meta.author ??= choice.detail;
  return { conversion, source: sheet.url };
}

export async function convertAddress(input: string): Promise<AddressResult> {
  const source = readSource(input);
  switch (source.kind) {
    case "problem":
      return source;
    case "search":
      return { groups: await searchThemes(source.query) };
    case "repo": {
      const files = await findGithubFiles(source);
      const [only] = files.choices;
      return files.choices.length === 1 && only !== undefined
        ? convertUrl(only.url, only)
        : { groups: [files] };
    }
    case "store": {
      const theme = await storeTheme(source.name);
      return theme === null ? { problem: "notFound" } : convertUrl(theme.url, theme);
    }
    case "css":
      return convertUrl(source.url);
  }
}

export async function fetchPicture(url: string): Promise<Blob> {
  if (url.startsWith("data:")) {
    const image = imageFromDataAddress(url);
    if (image === null) throw new Error("Expected an embedded raster image.");
    return image;
  }
  return (await fetchSource(url, "picture")).blob();
}
