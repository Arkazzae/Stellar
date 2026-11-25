/**
 * The text stages of reading a BetterDiscord theme: its META header, its
 * imports fetched and written in place, the addresses taken out before
 * anything evaluates it, and the `@container … style()` blocks an engine
 * without style queries cannot evaluate. Plain functions over strings, so
 * every one of them is tested without a browser.
 */

/** What a theme's `/** @name … *\/` header says about it. */
export interface ThemeMeta {
  name?: string;
  author?: string;
  version?: string;
  source?: string;
  description?: string;
}

const META_FIELDS = ["name", "author", "version", "source", "description"] as const;

/** The first block comment's `@field value` lines, when it is a META header. */
export function readMeta(css: string): ThemeMeta {
  const header = /^\s*\/\*\*([\s\S]*?)\*\//.exec(css)?.[1];
  if (header === undefined) return {};
  const meta: ThemeMeta = {};
  for (const line of header.split("\n")) {
    const match = /^\s*\*?\s*@(\w+)\s+(.+?)\s*$/.exec(line);
    const key = META_FIELDS.find((field) => field === match?.[1]);
    if (key !== undefined && match?.[2] !== undefined) meta[key] = match[2];
  }
  return meta;
}

/** One `@import`, where it stands in the text and what it names. */
interface ImportRule {
  start: number;
  end: number;
  url: string;
  /** What follows the address: `layer(…)`, `supports(…)`, a media query. */
  condition: string;
}

const IMPORT = /@import\s+(?:url\(\s*(["']?)([^"')]*)\1\s*\)|(["'])([^"']*)\3)\s*([^;]*);/gi;

function importsOf(css: string): ImportRule[] {
  return [...css.matchAll(IMPORT)].map((match) => ({
    start: match.index,
    end: match.index + match[0].length,
    url: (match[2] ?? match[4] ?? "").trim(),
    condition: (match[5] ?? "").trim(),
  }));
}

/** The families a Google Fonts stylesheet address asks for. */
export function googleFontFamilies(url: URL): string[] {
  return url.searchParams
    .getAll("family")
    .flatMap((value) => value.split("|"))
    .map((family) => family.split(":")[0]?.replace(/\+/g, " ").trim() ?? "")
    .filter((family) => family !== "");
}

/** Fetches a stylesheet's text, or rejects. */
export type FetchStylesheet = (url: string) => Promise<string | { text: string; url: string }>;

export interface InlineLimits {
  /** How deep imports may nest: the theme, its build, what that imports. */
  depth: number;
  /** How many characters all the stylesheets together may hold. */
  total: number;
}

export const INLINE_LIMITS: InlineLimits = { depth: 4, total: 2 * 1024 * 1024 };

export interface Inlined {
  css: string;
  /** Families asked of Google Fonts, which is never fetched. */
  googleFonts: string[];
  /** Imports left out while reading a bounded stylesheet tree. */
  skipped: { url: string; reason: "insecure" | "depth" | "size" | "failed" | "limit" | "cycle" }[];
  /** Every stylesheet read, the theme's own address first when it had one. */
  fetched: string[];
}

/**
 * The theme with every `@import` replaced by the stylesheet it names, fetched
 * over https and resolved against the importing sheet's address, to the
 * given depth and total size. A condition on an import (`layer`, `supports`,
 * a media query) wraps what it brings in the matching block, so the cascade
 * is the one the browser would have built.
 */
export async function inlineImports(
  css: string,
  base: string | null,
  fetchSheet: FetchStylesheet,
  limits: InlineLimits = INLINE_LIMITS,
): Promise<Inlined> {
  const result: Inlined = { css: "", googleFonts: [], skipped: [], fetched: base === null ? [] : [base] };
  let total = css.length;
  const sheets = new Map<string, ReturnType<FetchStylesheet>>();

  const expand = async (text: string, from: string | null, depth: number, ancestors: ReadonlySet<string>): Promise<string> => {
    text = text.replace(COMMENTS, "");
    let out = "";
    let at = 0;
    for (const rule of importsOf(text)) {
      out += absoluteAddresses(text.slice(at, rule.start), from);
      at = rule.end;
      let url: URL;
      try {
        url = new URL(rule.url, from ?? undefined);
      } catch {
        result.skipped.push({ url: rule.url, reason: "insecure" });
        continue;
      }
      if (url.hostname === "fonts.googleapis.com") {
        result.googleFonts.push(...googleFontFamilies(url));
        continue;
      }
      if (url.protocol !== "https:") {
        result.skipped.push({ url: url.href, reason: "insecure" });
        continue;
      }
      if (depth >= limits.depth) {
        result.skipped.push({ url: url.href, reason: "depth" });
        continue;
      }
      if (ancestors.has(url.href)) {
        result.skipped.push({ url: url.href, reason: "cycle" });
        continue;
      }
      let pending = sheets.get(url.href);
      if (pending === undefined) {
        if (sheets.size >= 16) {
          result.skipped.push({ url: url.href, reason: "limit" });
          continue;
        }
        pending = fetchSheet(url.href);
        sheets.set(url.href, pending);
      }
      let sheet: string | { text: string; url: string };
      try {
        sheet = await pending;
      } catch {
        result.skipped.push({ url: url.href, reason: "failed" });
        continue;
      }
      const body = typeof sheet === "string" ? sheet : sheet.text;
      const address = typeof sheet === "string" ? url.href : sheet.url;
      if (ancestors.has(address)) {
        result.skipped.push({ url: address, reason: "cycle" });
        continue;
      }
      if (total + body.length > limits.total) {
        result.skipped.push({ url: url.href, reason: "size" });
        continue;
      }
      total += body.length;
      result.fetched.push(address);
      out += wrapIn(rule.condition, await expand(body, address, depth + 1, new Set([...ancestors, url.href, address])));
    }
    return out + absoluteAddresses(text.slice(at), from);
  };

  result.css = await expand(css, base, 0, new Set(base === null ? [] : [base]));
  return result;
}

function wrapIn(condition: string, css: string): string {
  let wrapped = css;
  let rest = condition;
  const layer = /^layer(?:\(([^)]*)\))?\s*/.exec(rest);
  if (layer !== null) rest = rest.slice(layer[0].length);
  const supports = /^supports\(((?:[^()]|\([^()]*\))*)\)\s*/.exec(rest);
  if (supports !== null) rest = rest.slice(supports[0].length);
  if (rest.trim() !== "") wrapped = `@media ${rest.trim()} {\n${wrapped}\n}`;
  if (supports !== null) wrapped = `@supports (${supports[1]}) {\n${wrapped}\n}`;
  if (layer !== null) wrapped = `@layer ${layer[1]?.trim() ?? ""} {\n${wrapped}\n}`;
  return wrapped;
}

const COMMENTS = /\/\*[\s\S]*?\*\//g;
const URL_TOKEN = /url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*))\s*\)/gi;

function absoluteAddresses(css: string, base: string | null): string {
  return css.replace(URL_TOKEN, (token, double: string | undefined, single: string | undefined, bare: string | undefined) => {
    try {
      const url = new URL((double ?? single ?? bare ?? "").trim(), base ?? undefined);
      return url.protocol === "https:" ? `url("${url.href.replaceAll('"', "%22")}")` : token;
    } catch {
      return token;
    }
  });
}

/** Strip fetchable CSS. The optional callback turns image addresses into inert values. */
export function withoutAddresses(css: string, replaceAddress = (_address: string) => "none"): string {
  return removeBlocks(css.replace(COMMENTS, ""), /@font-face\s*\{/gi)
    .replace(/@import[^;]*;/gi, "")
    .replace(/(?:-webkit-)?image-set\(\s*(?:[^()]|\([^()]*\))*\)/gi, "none")
    .replace(URL_TOKEN, (_token, double: string | undefined, single: string | undefined, bare: string | undefined) =>
      replaceAddress((double ?? single ?? bare ?? "").trim()),
    );
}

/** Let the evaluator choose the winning wallpaper variable, including aliases and overrides. */
export function wallpaperValues(css: string) {
  const pictures = new Map<string, string>();
  const safeCss = withoutAddresses(css, (address) => {
    if (!address.startsWith("https://") || !/\.(?:jpe?g|png|webp|gif|avif)(?:[?#]|$)/i.test(address)) return "none";
    if (pictures.size >= 65536) return "none";
    // An inert image keeps the declaration valid on both custom properties
    // and actual backgrounds; RGB channels carry its index through CSSOM.
    const id = `rgb(1, ${pictures.size >> 8}, ${pictures.size & 255})`;
    pictures.set(id, address);
    return `linear-gradient(${id}, ${id})`;
  });
  const preferred = [
    "--background-image", "--app-bg", "--dplus-backdrop", "--wallpaper",
    "--theme-background-image", "--backgroundImage", "--background", "--bg-image", "--bg-img",
  ];
  const names = [...new Set([
    ...preferred,
    ...Array.from(safeCss.matchAll(/(--[\w-]*(?:background|backdrop|wallpaper|bg)[\w-]*)\s*:/gi), (match) => match[1]!),
  ])].slice(0, 80);
  const read = (values: ReadonlyMap<string, string>) => {
    for (const name of ["--stellarr-page-image", ...names]) {
      const id = values.get(name)?.match(/rgb\(1, \d+, \d+\)/)?.[0];
      const address = id === undefined ? undefined : pictures.get(id);
      if (address !== undefined) return address;
    }
    return null;
  };
  return { css: safeCss, names, read };
}

/** The index just past the block whose `{` is at `open`. */
function blockEnd(css: string, open: number): number {
  let depth = 0;
  for (let index = open; index < css.length; index += 1) {
    const char = css[index];
    if (char === "{") depth += 1;
    else if (char === "}" && --depth === 0) return index + 1;
  }
  return css.length;
}

function removeBlocks(css: string, start: RegExp): string {
  let out = "";
  let at = 0;
  for (const match of css.matchAll(start)) {
    if (match.index < at) continue;
    out += css.slice(at, match.index);
    at = blockEnd(css, match.index + match[0].length - 1);
  }
  return out + css.slice(at);
}

/** The custom properties `style()` tests name, by the container they ask. */
export function styleQueryNames(css: string): string[] {
  return [...new Set([...css.matchAll(/style\(\s*(--[\w-]+)\s*:/g)].map((match) => match[1] ?? ""))];
}

/** What a container's custom property is set to, by the container's name. */
export type DeclaredValue = (property: string, container: string | null) => string | undefined;

/**
 * For an engine that cannot evaluate container style queries: each
 * `@container [name] <condition>` whose condition is made only of
 * `style(--x: value)` tests joined by `and`, `or` and `not` is decided here,
 * against the values the theme itself declares, and replaced by its rules or
 * by nothing. A condition with any other test is left as it is and reported.
 */
export function unwrapStyleQueries(css: string, declared: DeclaredValue): { css: string; undecided: number } {
  let undecided = 0;
  const walk = (text: string): string => {
    let out = "";
    let at = 0;
    const rule = /@container\s+([^{]*)\{/g;
    let match: RegExpExecArray | null;
    while ((match = rule.exec(text)) !== null) {
      const open = match.index + match[0].length - 1;
      const end = blockEnd(text, open);
      out += text.slice(at, match.index);
      at = end;
      rule.lastIndex = end;
      const inner = walk(text.slice(open + 1, end - 1));
      const decision = decide(match[1] ?? "", declared);
      if (decision === null) {
        undecided += 1;
        out += `@container ${match[1]}{${inner}}`;
      } else if (decision) {
        out += inner;
      }
    }
    return out + text.slice(at);
  };
  return { css: walk(css), undecided };
}

/** True or false for a condition of style tests only; null for anything else. */
function decide(prelude: string, declared: DeclaredValue): boolean | null {
  const trimmed = prelude.trim();
  const named = /^([a-z_-][\w-]*)\s+(?=(?:not\s+)?(?:style\s*)?\()/i.exec(trimmed);
  const container = named === null || /^(not|and|or)$/i.test(named[1] ?? "") ? null : (named[1] ?? null);
  let condition = container === null ? trimmed : trimmed.slice(named?.[0].length ?? 0);
  if (!/style\(/.test(condition)) return null;

  const tests: boolean[] = [];
  condition = condition.replace(/style\(\s*(--[\w-]+)\s*:\s*((?:[^()]|\([^()]*\))*?)\s*\)/g, (_all, name: string, value: string) => {
    tests.push(declared(name, container)?.trim() === value.trim());
    return `$${tests.length - 1}`;
  });
  // What is left may only be the tests, joined and negated, in parentheses.
  const expression = condition
    .replace(/\band\b/gi, "&&")
    .replace(/\bor\b/gi, "||")
    .replace(/\bnot\b/gi, "!")
    .replace(/\s+/g, "");
  if (!/^[$\d()!&|]*$/.test(expression)) return null;
  return evaluate(expression, tests);
}

/** A tiny recursive reader for `!`, `&&`, `||` and parentheses over `$n` tests. */
function evaluate(expression: string, tests: boolean[]): boolean | null {
  let at = 0;
  const primary = (): boolean | null => {
    if (expression[at] === "!") {
      at += 1;
      const value = primary();
      return value === null ? null : !value;
    }
    if (expression[at] === "(") {
      at += 1;
      const value = or();
      if (expression[at] !== ")") return null;
      at += 1;
      return value;
    }
    const test = /^\$(\d+)/.exec(expression.slice(at));
    if (test === null) return null;
    at += test[0].length;
    return tests[Number(test[1])] ?? null;
  };
  const and = (): boolean | null => {
    let value = primary();
    while (expression.startsWith("&&", at)) {
      at += 2;
      const next = primary();
      value = value === null || next === null ? null : value && next;
    }
    return value;
  };
  const or = (): boolean | null => {
    let value = and();
    while (expression.startsWith("||", at)) {
      at += 2;
      const next = and();
      value = value === null || next === null ? null : value || next;
    }
    return value;
  };
  const value = or();
  return at === expression.length ? value : null;
}
