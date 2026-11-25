import { request } from "@/lib/api/transport";

const STORE = "https://api.betterdiscord.app/v3/store";
const GITHUB_API = "https://api.github.com";
const RAW = "https://raw.githubusercontent.com";
const MAX_RESULTS = 8;

type Kind = "css" | "catalog" | "picture";

class SourceError extends Error {
  constructor(readonly status: number) {
    super(`Theme source answered ${status}.`);
  }
}

export async function fetchSource(url: string, kind: Kind, signal?: AbortSignal): Promise<Response> {
  const timeout = AbortSignal.timeout(12_000);
  const response = await request(
    `/api/me/theme-source?${new URLSearchParams({ url, kind })}`,
    { referrerPolicy: "no-referrer" },
    signal === undefined ? timeout : AbortSignal.any([timeout, signal]),
  );
  if (!response.ok) throw new SourceError(response.status);
  return response;
}

export interface ThemeChoice {
  label: string;
  detail?: string;
  url: string;
}

export interface SourceGroup {
  provider: "betterDiscord" | "github" | "files";
  choices: ThemeChoice[];
  problem?: "unavailable" | "rateLimited";
}

export type SourceInput =
  | { kind: "css"; url: string }
  | { kind: "store"; name: string }
  | { kind: "repo"; repo: string; ref: string; directory: string }
  | { kind: "search"; query: string }
  | { kind: "problem"; problem: "notHttps" | "invalid" };

const ownerRepo = /^[\w.-]+\/[\w.-]+$/;
const themeFile = /\.theme\.css$/i;

export function readSource(input: string): SourceInput {
  const value = input.trim();
  if (!value || value.length > 4096) return { kind: "problem", problem: "invalid" };
  if (ownerRepo.test(value)) return { kind: "repo", repo: value.replace(/\.git$/, ""), ref: "HEAD", directory: "" };
  const candidate = /^(?:github\.com|(?:www\.)?betterdiscord\.app|raw\.githubusercontent\.com)\//i.test(value)
    ? `https://${value}`
    : value;
  if (!/^[\w+.-]+:\/\/|^(?:file|data|javascript|https?):/i.test(candidate)) {
    return value.length <= 100 ? { kind: "search", query: value } : { kind: "problem", problem: "invalid" };
  }

  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    return { kind: "problem", problem: "invalid" };
  }
  if (url.protocol !== "https:") return { kind: "problem", problem: "notHttps" };
  if (url.username || url.password) return { kind: "problem", problem: "invalid" };

  if (url.hostname === "betterdiscord.app" || url.hostname === "www.betterdiscord.app") {
    const pathname = url.pathname.replace(/\/$/, "");
    const match = /^\/themes?\/([^/]+)$/i.exec(pathname);
    const id = url.searchParams.get("id");
    if (/^\/(?:download|gh-redirect|theme)$/i.test(pathname) && id !== null && /^\d+$/.test(id)) {
      return { kind: "store", name: id };
    }
    if (match?.[1] !== undefined) {
      try {
        const name = decodeURIComponent(match[1]);
        if (!name.includes("/")) return { kind: "store", name };
      } catch {
        return { kind: "problem", problem: "invalid" };
      }
    }
    if (pathname === "/themes") return { kind: "search", query: url.searchParams.get("q")?.slice(0, 100) ?? "" };
    return { kind: "problem", problem: "invalid" };
  }

  if (url.hostname === "github.com") {
    const [owner, name, section, ...rest] = url.pathname.split("/").slice(1);
    const repo = `${owner ?? ""}/${name?.replace(/\.git$/, "") ?? ""}`;
    if (!ownerRepo.test(repo)) return { kind: "problem", problem: "invalid" };
    if ((section === "blob" || section === "raw") && rest.length >= 2 && /\.css$/i.test(rest.at(-1) ?? "")) {
      return { kind: "css", url: `${RAW}/${repo}/${rest.join("/")}` };
    }
    if (!section || section === "tree") {
      try {
        const [ref = "HEAD", ...directory] = rest.map(decodeURIComponent);
        return { kind: "repo", repo, ref, directory: directory.filter(Boolean).join("/") };
      } catch {
        return { kind: "problem", problem: "invalid" };
      }
    }
    return { kind: "problem", problem: "invalid" };
  }

  url.hash = "";
  return { kind: "css", url: url.href };
}

interface StoreEntry {
  type?: string;
  name?: string;
  latest_source_url?: string;
  author?: { display_name?: string };
  downloads?: number;
  tags?: string[];
}

function storeChoice(entry: StoreEntry): ThemeChoice | null {
  if (entry.type !== "theme" || typeof entry.name !== "string" || typeof entry.latest_source_url !== "string") return null;
  const source = readSource(entry.latest_source_url);
  if (source.kind !== "css" || !themeFile.test(new URL(source.url).pathname)) return null;
  const author = entry.author?.display_name;
  return { label: entry.name, ...(typeof author === "string" ? { detail: author } : {}), url: source.url };
}

export async function storeTheme(name: string): Promise<ThemeChoice | null> {
  try {
    const response = await fetchSource(`${STORE}/${encodeURIComponent(name)}`, "catalog");
    return storeChoice(await response.json() as StoreEntry);
  } catch (error) {
    if (error instanceof SourceError && error.status === 404) return null;
    throw error;
  }
}

let catalog: Promise<StoreEntry[]> | undefined;
let catalogExpires = 0;

function storeCatalog(): Promise<StoreEntry[]> {
  if (catalog !== undefined && Date.now() < catalogExpires) return catalog;
  catalogExpires = Date.now() + 5 * 60_000;
  catalog = fetchSource(`${STORE}/themes`, "catalog")
    .then(async (response) => await response.json() as StoreEntry[])
    .catch((error: unknown) => {
      catalog = undefined;
      throw error;
    });
  return catalog;
}

async function storeSearch(query: string): Promise<ThemeChoice[]> {
  const normalise = (text: string) => text.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
  const term = normalise(query);
  return (await storeCatalog())
    .map((entry) => ({ entry, name: normalise(entry.name ?? ""), choice: storeChoice(entry) }))
    .filter(({ entry, name, choice }) => choice !== null && (
      name.includes(term) || entry.tags?.some((tag) => normalise(tag).includes(term)) === true
    ))
    .sort((a, b) => Number(b.name === term) - Number(a.name === term) ||
      Number(b.name.startsWith(term)) - Number(a.name.startsWith(term)) ||
      (b.entry.downloads ?? 0) - (a.entry.downloads ?? 0))
    .slice(0, query === "" ? undefined : MAX_RESULTS)
    .flatMap(({ choice }) => choice === null ? [] : [choice]);
}

async function githubSearch(query: string): Promise<ThemeChoice[]> {
  const params = new URLSearchParams({ q: `${query} discord in:name,description,readme`, per_page: String(MAX_RESULTS) });
  const response = await fetchSource(`${GITHUB_API}/search/repositories?${params}`, "catalog");
  const { items } = await response.json() as { items: { full_name: string; description: string | null }[] };
  return items.filter((item) => ownerRepo.test(item.full_name)).map((item) => ({
    label: item.full_name,
    ...(item.description === null ? {} : { detail: item.description }),
    url: `https://github.com/${item.full_name}`,
  }));
}

async function searchGroup(provider: SourceGroup["provider"], search: Promise<ThemeChoice[]>): Promise<SourceGroup> {
  try {
    return { provider, choices: await search };
  } catch (error) {
    return { provider, choices: [], problem: error instanceof SourceError && error.status === 429 ? "rateLimited" : "unavailable" };
  }
}

export function searchThemes(query: string): Promise<SourceGroup[]> {
  const catalogResults = searchGroup("betterDiscord", storeSearch(query));
  return Promise.all(query === "" ? [catalogResults] : [catalogResults, searchGroup("github", githubSearch(query))]);
}

interface TreeEntry {
  path: string;
  type: string;
  mode: string;
  sha: string;
}

async function githubTree(repo: string, ref: string, recursive = false) {
  const response = await fetchSource(
    `${GITHUB_API}/repos/${repo}/git/trees/${encodeURIComponent(ref)}${recursive ? "?recursive=1" : ""}`,
    "catalog",
  );
  return await response.json() as { tree: TreeEntry[]; truncated: boolean };
}

function themePaths(tree: TreeEntry[], prefix: string): string[] {
  return tree
    .filter(({ path, type, mode }) => type === "blob" && mode !== "120000" && themeFile.test(path))
    .map(({ path }) => prefix + path);
}

async function listThemePaths(repo: string, ref: string, directory: string): Promise<string[]> {
  let subtree = ref;
  let prefix = "";
  for (const segment of directory.split("/").filter(Boolean)) {
    const { tree } = await githubTree(repo, subtree);
    const dir = tree.find(({ path, type }) => path === segment && type === "tree");
    if (dir === undefined) return [];
    subtree = dir.sha;
    prefix += `${segment}/`;
  }

  return scanTree(repo, subtree, prefix);
}

async function scanTree(repo: string, ref: string, prefix: string): Promise<string[]> {
  const root = await githubTree(repo, ref, true).catch((error: unknown) => {
    if (error instanceof SourceError && error.status === 502) return null;
    throw error;
  });
  if (root !== null && !root.truncated) return themePaths(root.tree, prefix);

  // Huge addon collections can exceed GitHub's recursive response budget.
  const shallow = await githubTree(repo, ref);
  const files = new Set([...themePaths(root?.tree ?? [], prefix), ...themePaths(shallow.tree, prefix)]);
  const themeDirectory = /(?:^|\/)themes?\/$/i.test(prefix);
  const candidates = shallow.tree.filter(({ type, path }) => type === "tree" && (
    themeDirectory ? /^[a-z0-9]/i.test(path) : /^(?:themes?|dist|src|betterdiscord|bd|css|clients|styles|build)$/i.test(path)
  )).slice(0, themeDirectory ? 12 : 4);

  for (let i = 0; i < candidates.length; i += 4) {
    const results = await Promise.allSettled(candidates.slice(i, i + 4).map(async ({ path, sha }) => {
      const childPrefix = `${prefix}${path}/`;
      if (!themeDirectory && /^themes?$/i.test(path)) return scanTree(repo, sha, childPrefix);
      const tree = await githubTree(repo, sha, !themeDirectory);
      return themePaths(tree.tree, childPrefix);
    }));
    for (const result of results) {
      if (result.status === "fulfilled") result.value.forEach((path) => files.add(path));
    }
  }
  return [...files].sort();
}

async function githubFiles(repo: string, ref: string, directory: string): Promise<ThemeChoice[]> {
  const files = await listThemePaths(repo, ref, directory);
  const prefix = directory ? `${directory}/` : "";
  return files.map((path) => ({
    label: path.slice(prefix.length),
    url: `${RAW}/${repo}/${encodeURIComponent(ref)}/${path.split("/").map(encodeURIComponent).join("/")}`,
  }));
}

export async function findGithubFiles(input: Extract<SourceInput, { kind: "repo" }>): Promise<SourceGroup> {
  const files = await searchGroup("files", githubFiles(input.repo, input.ref, input.directory));
  if (files.choices.length > 0 || input.ref !== "HEAD" || input.directory !== "") return files;

  // Some projects publish their install file from a release branch.
  try {
    const prefix = `${RAW}/${input.repo}/`.toLowerCase();
    const releases = (await storeCatalog()).flatMap((entry) => {
      const choice = storeChoice(entry);
      return choice !== null && choice.url.toLowerCase().startsWith(prefix) ? [choice] : [];
    });
    return releases.length === 0 ? files : { provider: "files", choices: releases };
  } catch {
    return files;
  }
}
