import { t } from "../../i18n";
import { formatNumber } from "../../i18n/format";
import { CockpitApiError, cockpitRequest } from "../../lib/api";

export interface ActivityManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  /** What the activity may ask a participant's device for, such as `media.cameraTile`. */
  permissions: string[];
  /** Hosts outside the instance the activity talks to, and what for. */
  connects: { host: string; why: string }[];
}

export interface ActivityArtwork {
  icon: string | null;
  cover: string | null;
}

/** One published version of an activity, as the server read it from its release. */
export interface ActivityRelease {
  artwork?: ActivityArtwork;
  /** The repository, such as https://github.com/owner/name. */
  source: string;
  /** The page of the release this version came from. */
  release: string;
  /** The commit the release was built from. */
  revision: string;
  digest: string;
  bytes: number;
  manifest: ActivityManifest;
}

export interface InstalledActivity extends ActivityRelease {
  install: string;
  enabled: boolean;
}

/** An activity a repository offers, before its package is read. */
export interface ActivityChoice {
  release: string;
  id: string;
  name: string;
  version: string;
  bytes: number;
}

export interface ActivityLibrary {
  activities: InstalledActivity[];
  /** False when the instance cannot serve activities as it is set up. */
  hosting: boolean;
}

const BASE = "/api/cockpit/activities";
const post = <T>(path: string, body: unknown) => cockpitRequest<T>(`${BASE}/${path}`, { method: "POST", body });

export const activityApi = {
  library: (signal?: AbortSignal) => cockpitRequest<ActivityLibrary>(BASE, {}, signal),
  /** What a repository or a release link offers. */
  discover: (url: string) => post<{ source: string; entries: ActivityChoice[] }>("discover", { url }),
  /**
   * Reads one activity's package without installing it. With `update`, the
   * installation to compare with, the answer is `current` unless the release
   * is newer than what is installed.
   */
  review: (url: string, id: string, update?: string) =>
    post<ActivityRelease | { current: true }>("review", { url, id, update }),
  /** Installs exactly the reviewed package; the server refuses when the release changed since. */
  install: (reviewed: ActivityRelease, update?: string) =>
    post<ActivityRelease>("install", {
      url: reviewed.release,
      id: reviewed.manifest.id,
      digest: reviewed.digest,
      revision: reviewed.revision,
      update,
    }),
  manage: (install: string, action: "enable" | "disable" | "uninstall") =>
    post<{ ok: true }>(`${encodeURIComponent(install)}/manage`, { action }),
};

/** Why a request failed, in the server's words for the activity where it gave them. */
export function explain(cause: unknown): string {
  if (cause instanceof CockpitApiError) return cause.errors.activity?.[0] ?? cause.message;
  return cause instanceof Error ? cause.message : t("activities.failed");
}

/** A permission as a sentence, for the review before an install. */
export function permissionLabel(name: string): string {
  if (name === "media.microphoneLevel") return t("activities.mic");
  if (name === "media.cameraTile") return t("activities.camera");
  if (name === "media.gifs") return t("activities.gifs");
  return name;
}

/** A permission as a word or two, for a table cell. */
export function permissionShort(name: string): string {
  if (name === "media.microphoneLevel") return t("activities.short.mic");
  if (name === "media.cameraTile") return t("activities.short.camera");
  if (name === "media.gifs") return t("activities.short.gifs");
  return name;
}

/** A package's size in megabytes, always to one decimal so a column of them lines up. */
export function formatSize(bytes: number): string {
  return formatNumber(bytes / 1024 / 1024, {
    style: "unit",
    unit: "megabyte",
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  });
}

/** `owner/name` of a GitHub repository address; any other address as it is. */
export function repositoryName(source: string): string {
  return source.replace(/^https:\/\/github\.com\//, "");
}

/** The tag a release page is for; the whole address when it names none. */
export function releaseTag(release: string): string {
  const tag = release.split("/releases/tag/")[1];
  if (!tag) return release;
  try {
    return decodeURIComponent(tag);
  } catch {
    return tag;
  }
}
