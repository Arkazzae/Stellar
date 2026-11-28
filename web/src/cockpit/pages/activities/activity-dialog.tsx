import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  cx,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Spinner,
} from "@stellarr/ui";
import { useState, type FormEvent, type ReactNode } from "react";

import { t } from "../../i18n";
import {
  activityApi,
  explain,
  formatSize,
  permissionLabel,
  releaseTag,
  repositoryName,
  type ActivityArtwork,
  type ActivityChoice,
  type ActivityRelease,
  type InstalledActivity,
} from "./library";

/** An activity's icon, or its initial where the package ships none. */
export function ActivityIcon({
  artwork,
  name,
  className,
}: {
  artwork?: ActivityArtwork;
  name: string;
  className?: string;
}) {
  return (
    <span
      aria-hidden="true"
      className={cx(
        "flex size-9 shrink-0 items-center justify-center overflow-hidden rounded-md border bg-background text-sm font-semibold",
        className,
      )}
    >
      {artwork?.icon ? <img src={artwork.icon} alt="" loading="lazy" className="size-full object-cover" /> : name.slice(0, 1)}
    </span>
  );
}

/** One attempt at a time: whether it is running, and why the last one failed. */
function useAttempt() {
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  async function attempt(action: () => Promise<void>) {
    setPending(true);
    setFailure(null);
    try {
      await action();
    } catch (cause) {
      setFailure(explain(cause));
    } finally {
      setPending(false);
    }
  }

  return { pending, failure, attempt };
}

function Failure({ message }: { message: string | null }) {
  if (!message) return null;
  return (
    <Alert variant="destructive">
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}

const externalLink = "text-brand underline-offset-4 hover:underline";

/** The address step: a repository or release link, then the activity to take when it offers several. */
export function AddStep({ onReview, onCancel }: { onReview: (release: ActivityRelease) => void; onCancel: () => void }) {
  const [url, setUrl] = useState("");
  const [choices, setChoices] = useState<ActivityChoice[] | null>(null);
  const { pending, failure, attempt } = useAttempt();

  async function read(choice: ActivityChoice) {
    const result = await activityApi.review(choice.release, choice.id);
    if (!("current" in result)) onReview(result);
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void attempt(async () => {
      setChoices(null);
      const found = await activityApi.discover(url);
      const only = found.entries.length === 1 ? found.entries[0] : undefined;
      // A repository with one activity goes straight to its review.
      if (only) await read(only);
      else setChoices(found.entries);
    });
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("activities.add")}</DialogTitle>
        <DialogDescription>{t("activities.intro")}</DialogDescription>
      </DialogHeader>
      <form className="space-y-4" onSubmit={submit}>
        <div className="space-y-2">
          <Label htmlFor="activity-url">{t("activities.url")}</Label>
          <Input
            id="activity-url"
            required
            type="url"
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            placeholder="https://github.com/owner/repository"
            autoComplete="off"
            autoFocus
          />
        </div>
        {choices ? (
          <div className="space-y-2">
            <h3 className="text-sm font-medium">{t("activities.choose")}</h3>
            <ul className="divide-y rounded-md border">
              {choices.map((choice) => (
                <li key={choice.id}>
                  <button
                    type="button"
                    disabled={pending}
                    onClick={() => void attempt(() => read(choice))}
                    className="flex w-full items-baseline gap-3 px-3 py-2.5 text-left text-sm outline-none hover:bg-accent focus-visible:bg-accent disabled:opacity-50"
                  >
                    <span className="mr-auto font-medium">{choice.name}</span>
                    <span className="tabular-nums">{choice.version}</span>
                    <span className="w-20 text-right text-muted-foreground tabular-nums">
                      {formatSize(choice.bytes)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        <Failure message={failure} />
        <DialogFooter>
          <Button type="button" variant="secondary" onClick={onCancel}>
            {t("activities.cancel")}
          </Button>
          <Button type="submit" disabled={pending}>
            {pending ? <Spinner /> : null}
            {t("activities.find")}
          </Button>
        </DialogFooter>
      </form>
    </>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-6 py-2">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate text-right">{children}</dd>
    </div>
  );
}

/**
 * What a version is and what it may reach: its release, and the permissions
 * and hosts it asks for. Beside an installed version, `previous`, each
 * difference is named.
 */
function ReleaseFacts({ release, previous }: { release: ActivityRelease; previous: InstalledActivity | null }) {
  const { manifest } = release;
  const before = previous?.manifest;
  const droppedPermissions = before?.permissions.filter((name) => !manifest.permissions.includes(name)) ?? [];
  const droppedHosts = before?.connects.filter((old) => !manifest.connects.some((next) => next.host === old.host)) ?? [];

  return (
    <>
      {release.artwork?.cover ? (
        <img src={release.artwork.cover} alt="" loading="lazy" className="h-28 w-full rounded-md object-cover" />
      ) : null}
      <dl className="divide-y border-y text-sm">
        <Fact label={t("activities.col.version")}>
          <span className="tabular-nums">
            {before ? `${before.version} → ` : ""}
            {manifest.version}
          </span>
        </Fact>
        <Fact label={t("activities.col.source")}>
          <a href={release.source} target="_blank" rel="noreferrer" className={externalLink}>
            {repositoryName(release.source)}
          </a>
        </Fact>
        <Fact label={t("activities.release")}>
          <a href={release.release} target="_blank" rel="noreferrer" className={externalLink}>
            {releaseTag(release.release)}
          </a>
        </Fact>
        <Fact label={t("activities.commit")}>
          <span className="font-mono text-xs" title={release.revision}>
            {release.revision.slice(0, 7)}
          </span>
        </Fact>
        <Fact label={t("activities.col.size")}>
          <span className="tabular-nums">{formatSize(release.bytes)}</span>
        </Fact>
      </dl>

      <section className="space-y-2 text-sm">
        <h3 className="font-medium">{t("activities.permissions")}</h3>
        {manifest.permissions.length > 0 || droppedPermissions.length > 0 ? (
          <ul className="space-y-2">
            {manifest.permissions.map((name) => (
              <li key={name} className="flex items-start justify-between gap-3">
                <span>{permissionLabel(name)}</span>
                {before && !before.permissions.includes(name) ? (
                  <Badge variant="warning">{t("activities.added")}</Badge>
                ) : null}
              </li>
            ))}
            {droppedPermissions.map((name) => (
              <li key={name} className="flex items-start justify-between gap-3 text-muted-foreground">
                <span>{permissionLabel(name)}</span>
                <Badge variant="outline">{t("activities.removed")}</Badge>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-muted-foreground">{t("activities.none")}</p>
        )}
      </section>

      <section className="space-y-2 text-sm">
        <h3 className="font-medium">{t("activities.hosts")}</h3>
        {manifest.connects.length > 0 || droppedHosts.length > 0 ? (
          <ul className="space-y-2">
            {manifest.connects.map(({ host, why }) => (
              <li key={host} className="flex items-start justify-between gap-3">
                <span className="min-w-0">
                  <span className="block font-medium break-all">{host}</span>
                  <span className="block text-muted-foreground">{why}</span>
                </span>
                {before && !before.connects.some((old) => old.host === host && old.why === why) ? (
                  <Badge variant="warning">{t("activities.changed")}</Badge>
                ) : null}
              </li>
            ))}
            {droppedHosts.map(({ host }) => (
              <li key={host} className="flex items-start justify-between gap-3 text-muted-foreground">
                <span className="min-w-0 break-all">{host}</span>
                <Badge variant="outline">{t("activities.removed")}</Badge>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-muted-foreground">{t("activities.none")}</p>
        )}
      </section>
    </>
  );
}

function ReleaseHeader({ release }: { release: ActivityRelease }) {
  return (
    <DialogHeader>
      <div className="flex items-center gap-3">
        <ActivityIcon artwork={release.artwork} name={release.manifest.name} className="size-11 text-base" />
        <DialogTitle>{release.manifest.name}</DialogTitle>
      </div>
      <DialogDescription>{release.manifest.description}</DialogDescription>
    </DialogHeader>
  );
}

/** The review before a first install, or before an update of `current`. */
export function ReviewStep({
  release,
  current,
  onConfirm,
  onCancel,
}: {
  release: ActivityRelease;
  current: InstalledActivity | null;
  /** Installs the release; rejects with the reason when the server refuses. */
  onConfirm: () => Promise<void>;
  onCancel: () => void;
}) {
  const { pending, failure, attempt } = useAttempt();

  return (
    <>
      <ReleaseHeader release={release} />
      <ReleaseFacts release={release} previous={current} />
      <Failure message={failure} />
      <DialogFooter>
        {/* Focus starts on the way out, so Enter alone never installs. */}
        <Button type="button" variant="secondary" onClick={onCancel} autoFocus>
          {t("activities.cancel")}
        </Button>
        <Button type="button" disabled={pending} onClick={() => void attempt(onConfirm)}>
          {pending ? <Spinner /> : null}
          {current ? t("activities.update") : t("activities.install")}
        </Button>
      </DialogFooter>
    </>
  );
}

/** What is installed, in full: the table has room for a summary only. */
export function DetailsStep({ entry, onClose }: { entry: InstalledActivity; onClose: () => void }) {
  return (
    <>
      <ReleaseHeader release={entry} />
      <ReleaseFacts release={entry} previous={null} />
      <DialogFooter>
        <Button type="button" variant="secondary" onClick={onClose} autoFocus>
          {t("common.close")}
        </Button>
      </DialogFooter>
    </>
  );
}

export function UninstallStep({
  entry,
  onConfirm,
  onCancel,
}: {
  entry: InstalledActivity;
  onConfirm: () => Promise<void>;
  onCancel: () => void;
}) {
  const { pending, failure, attempt } = useAttempt();

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("activities.uninstall.title", { name: entry.manifest.name })}</DialogTitle>
        <DialogDescription>{t("activities.confirmRemove")}</DialogDescription>
      </DialogHeader>
      <Failure message={failure} />
      <DialogFooter>
        <Button type="button" variant="secondary" onClick={onCancel} autoFocus>
          {t("activities.cancel")}
        </Button>
        <Button type="button" variant="danger-solid" disabled={pending} onClick={() => void attempt(onConfirm)}>
          {pending ? <Spinner /> : null}
          {t("activities.uninstall")}
        </Button>
      </DialogFooter>
    </>
  );
}
