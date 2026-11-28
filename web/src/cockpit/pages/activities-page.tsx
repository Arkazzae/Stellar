import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  Card,
  cx,
  Dialog,
  DialogContent,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Spinner,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@stellarr/ui";
import { useState } from "react";

import { CardTitleRow } from "../components/card-title-row";
import { Icon } from "../components/icons/icon";
import { PageHeader } from "../components/page-header";
import { t } from "../i18n";
import { formatNumber, formatTime } from "../i18n/format";
import { CockpitApiError } from "../lib/api";
import { useCockpitResource } from "../lib/use-cockpit-resource";
import { ActivityIcon, AddStep, DetailsStep, ReviewStep, UninstallStep } from "./activities/activity-dialog";
import {
  activityApi,
  explain,
  permissionShort,
  repositoryName,
  type ActivityRelease,
  type InstalledActivity,
} from "./activities/library";

/** What the dialog shows. Its content stays while the dialog fades out, so the step is kept apart from `open`. */
type Flow =
  | { step: "add" }
  | { step: "review"; release: ActivityRelease; current: InstalledActivity | null }
  | { step: "details"; entry: InstalledActivity }
  | { step: "uninstall"; entry: InstalledActivity };

/** What the last look at an installation's repository found. No entry means nobody has looked yet. */
type UpdateState =
  | { kind: "checking" }
  | { kind: "current"; at: number }
  | { kind: "available"; at: number; release: ActivityRelease }
  | { kind: "failed"; message: string };

interface Notice {
  message: string;
  error: boolean;
}

/**
 * How long a cell's text may run before it is cut, with the whole of it in its
 * title. Cells do not wrap, so these keep the table inside a laptop's window;
 * a wider one gives them more room.
 */
const fit = { wide: "max-w-48 min-[100rem]:max-w-64", narrow: "max-w-40 min-[100rem]:max-w-52" };

/** A finding is about one installed version: after an update it no longer applies. */
const versionKey = (install: string, digest: string) => `${install}:${digest}`;

function CheckedAt({ at }: { at: number }) {
  return <p className="text-xs text-muted-foreground">{t("activities.update.checkedAt", { time: formatTime(at) })}</p>;
}

function UpdateCell({ state, onReview }: { state: UpdateState | undefined; onReview: () => void }) {
  if (!state) return <span className="text-muted-foreground">{t("activities.update.unchecked")}</span>;
  switch (state.kind) {
    case "checking":
      return (
        <span role="status" className="flex items-center gap-2 text-muted-foreground">
          <Spinner />
          {t("activities.update.checking")}
        </span>
      );
    case "current":
      return (
        <>
          <p>{t("activities.update.current")}</p>
          <CheckedAt at={state.at} />
        </>
      );
    case "available":
      return (
        <div className="flex items-center gap-3">
          <div>
            <Badge variant="warning" className="tabular-nums">
              {t("activities.update.available", { version: state.release.manifest.version })}
            </Badge>
            <CheckedAt at={state.at} />
          </div>
          <Button size="sm" variant="secondary" onClick={onReview}>
            {t("activities.update")}
          </Button>
        </div>
      );
    case "failed":
      return (
        <>
          <p className="text-danger">{t("activities.update.failed")}</p>
          <p className={cx(fit.narrow, "truncate text-xs text-muted-foreground")} title={state.message}>
            {state.message}
          </p>
        </>
      );
  }
}

/** The permissions in a word each, and under them the hosts the activity reaches. */
function AccessCell({ entry }: { entry: InstalledActivity }) {
  const { permissions, connects } = entry.manifest;
  if (permissions.length === 0 && connects.length === 0) {
    return <span className="text-muted-foreground">{t("activities.none")}</span>;
  }
  const granted = permissions.map(permissionShort).join(", ");
  const hosts = connects.map(({ host }) => host).join(", ");
  return (
    <>
      {granted ? (
        <p className={cx(fit.narrow, "truncate")} title={granted}>
          {granted}
        </p>
      ) : (
        <p className="text-muted-foreground">{t("activities.noPermissions")}</p>
      )}
      {hosts ? (
        <p className={cx(fit.narrow, "truncate text-xs text-muted-foreground")} title={hosts}>
          {hosts}
        </p>
      ) : null}
    </>
  );
}

interface ActivityRowProps {
  entry: InstalledActivity;
  update: UpdateState | undefined;
  toggling: boolean;
  onToggle: (enabled: boolean) => void;
  onCheck: () => void;
  onReview: (release: ActivityRelease) => void;
  onDetails: () => void;
  onUninstall: () => void;
}

function ActivityRow({ entry, update, toggling, onToggle, onCheck, onReview, onDetails, onUninstall }: ActivityRowProps) {
  const { manifest } = entry;
  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-3">
          <ActivityIcon artwork={entry.artwork} name={manifest.name} />
          <div className="min-w-0">
            <p className="font-medium">{manifest.name}</p>
            <p className={cx(fit.wide, "truncate text-xs text-muted-foreground")} title={manifest.description}>
              {manifest.description}
            </p>
          </div>
        </div>
      </TableCell>
      <TableCell className="tabular-nums">{manifest.version}</TableCell>
      <TableCell>
        <UpdateCell
          state={update}
          onReview={() => {
            if (update?.kind === "available") onReview(update.release);
          }}
        />
      </TableCell>
      <TableCell>
        <a
          href={entry.source}
          target="_blank"
          rel="noreferrer"
          title={entry.source}
          className={cx(fit.narrow, "block truncate text-brand underline-offset-4 hover:underline")}
        >
          {repositoryName(entry.source)}
        </a>
      </TableCell>
      <TableCell>
        <AccessCell entry={entry} />
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-2.5">
          <Switch
            checked={entry.enabled}
            disabled={toggling}
            onCheckedChange={onToggle}
            aria-label={t("activities.toggle", { name: manifest.name })}
          />
          <span className={entry.enabled ? undefined : "text-muted-foreground"}>
            {entry.enabled ? t("activities.enabled") : t("activities.disabled")}
          </span>
        </div>
      </TableCell>
      <TableCell className="text-right">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="quiet" size="icon-sm" aria-label={t("activities.actions", { name: manifest.name })}>
              <Icon name="dots" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onDetails}>{t("activities.details")}</DropdownMenuItem>
            <DropdownMenuItem disabled={update?.kind === "checking"} onSelect={onCheck}>
              {t("activities.check")}
            </DropdownMenuItem>
            <DropdownMenuItem variant="destructive" onSelect={onUninstall}>
              {t("activities.uninstall")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </TableRow>
  );
}

export function ActivitiesPage() {
  const { data, error, loading, refresh } = useCockpitResource(activityApi.library, t("activities.failed"));
  const [dialog, setDialog] = useState<{ open: boolean; flow: Flow }>({ open: false, flow: { step: "add" } });
  const [updates, setUpdates] = useState<Record<string, UpdateState>>({});
  const [notice, setNotice] = useState<Notice | null>(null);
  const [toggling, setToggling] = useState<string | null>(null);
  const [checkingAll, setCheckingAll] = useState(false);
  const activities = data?.activities ?? [];
  const { open, flow } = dialog;

  const show = (next: Flow) => setDialog({ open: true, flow: next });
  const close = () => setDialog((current) => ({ ...current, open: false }));
  /** Closes the dialog a finished request belongs to, and no other opened since. */
  const closeAfter = (done: Flow) =>
    setDialog((current) => (current.flow === done ? { ...current, open: false } : current));
  const found = (install: string, digest: string, state: UpdateState) =>
    setUpdates((current) => ({ ...current, [versionKey(install, digest)]: state }));

  /** Looks for a newer release of one installation. Resolves to whether looking at the next one makes sense. */
  async function check(entry: InstalledActivity): Promise<boolean> {
    found(entry.install, entry.digest, { kind: "checking" });
    try {
      const result = await activityApi.review(entry.source, entry.manifest.id, entry.install);
      found(
        entry.install,
        entry.digest,
        "current" in result
          ? { kind: "current", at: Date.now() }
          : { kind: "available", at: Date.now(), release: result },
      );
      return true;
    } catch (cause) {
      found(entry.install, entry.digest, { kind: "failed", message: explain(cause) });
      // GitHub's rate limit and a server out of reach fail every repository alike.
      return !(cause instanceof CockpitApiError && (cause.status === 429 || cause.status === 0));
    }
  }

  /** One repository at a time: the server reads two releases at once at most, and GitHub limits requests. */
  async function checkAll() {
    setCheckingAll(true);
    for (const entry of activities) {
      if (!(await check(entry))) break;
    }
    setCheckingAll(false);
  }

  async function toggle(entry: InstalledActivity, enabled: boolean) {
    setToggling(entry.install);
    try {
      await activityApi.manage(entry.install, enabled ? "enable" : "disable");
      await refresh();
      setNotice(null);
    } catch (cause) {
      setNotice({ message: explain(cause), error: true });
    } finally {
      setToggling(null);
    }
  }

  async function install(reviewing: Extract<Flow, { step: "review" }>) {
    const { release, current } = reviewing;
    await activityApi.install(release, current?.install);
    const names = { name: release.manifest.name, version: release.manifest.version };
    // The version just installed is the newest its repository has.
    if (current) found(current.install, release.digest, { kind: "current", at: Date.now() });
    setNotice({ message: current ? t("activities.updated", names) : t("activities.installed", names), error: false });
    closeAfter(reviewing);
    await refresh();
  }

  async function uninstall(removing: Extract<Flow, { step: "uninstall" }>) {
    await activityApi.manage(removing.entry.install, "uninstall");
    setNotice({ message: t("activities.uninstalled", { name: removing.entry.manifest.name }), error: false });
    closeAfter(removing);
    await refresh();
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-6">
      <PageHeader
        title={t("nav.activities")}
        description={t("activities.intro")}
        actions={
          <>
            <Button variant="secondary" disabled={checkingAll || activities.length === 0} onClick={() => void checkAll()}>
              {checkingAll ? <Spinner /> : null}
              {t("activities.checkAll")}
            </Button>
            <Button onClick={() => show({ step: "add" })}>{t("activities.add")}</Button>
          </>
        }
      />

      {error ? (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}
      {data && !data.hosting ? (
        <Alert variant="destructive">
          <AlertDescription>{t("activities.hosting")}</AlertDescription>
        </Alert>
      ) : null}
      {notice ? (
        <Alert variant={notice.error ? "destructive" : "default"}>
          <AlertDescription>{notice.message}</AlertDescription>
        </Alert>
      ) : null}

      {!data && loading ? (
        <div role="status" className="flex min-h-48 items-center justify-center gap-2 text-sm text-muted-foreground">
          <Spinner />
          {t("activities.loading")}
        </div>
      ) : null}

      {data ? (
        <Card className="min-h-0 flex-1 gap-0 py-0">
          <CardTitleRow icon="game" title={t("activities.list")}>
            <span className="text-sm text-muted-foreground">{formatNumber(activities.length)}</span>
          </CardTitleRow>
          <div className="relative min-h-0 flex-1 overflow-auto px-3 pb-2 sm:px-5">
            <Table containerClassName="overflow-visible">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("activities.col.activity")}</TableHead>
                  <TableHead>{t("activities.col.version")}</TableHead>
                  <TableHead>{t("activities.col.update")}</TableHead>
                  <TableHead>{t("activities.col.source")}</TableHead>
                  <TableHead>{t("activities.col.access")}</TableHead>
                  <TableHead>{t("activities.col.state")}</TableHead>
                  <TableHead>
                    <span className="sr-only">{t("activities.col.actions")}</span>
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {activities.map((entry) => (
                  <ActivityRow
                    key={entry.install}
                    entry={entry}
                    update={updates[versionKey(entry.install, entry.digest)]}
                    toggling={toggling === entry.install}
                    onToggle={(enabled) => void toggle(entry, enabled)}
                    onCheck={() => void check(entry)}
                    onReview={(release) => show({ step: "review", release, current: entry })}
                    onDetails={() => show({ step: "details", entry })}
                    onUninstall={() => show({ step: "uninstall", entry })}
                  />
                ))}
                {activities.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="h-28 text-center whitespace-normal text-muted-foreground">
                      {t("activities.empty")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
        </Card>
      ) : null}

      <Dialog open={open} onOpenChange={(next) => !next && close()}>
        <DialogContent
          closeLabel={t("common.close")}
          className={flow.step === "uninstall" ? undefined : "max-h-[85dvh] overflow-y-auto sm:max-w-xl"}
        >
          {flow.step === "add" ? (
            <AddStep
              // The review a closed dialog was waiting for must not open it again.
              onReview={(release) =>
                setDialog((current) =>
                  current.open && current.flow === flow
                    ? { open: true, flow: { step: "review", release, current: null } }
                    : current,
                )
              }
              onCancel={close}
            />
          ) : flow.step === "review" ? (
            <ReviewStep
              release={flow.release}
              current={flow.current}
              onConfirm={() => install(flow)}
              onCancel={close}
            />
          ) : flow.step === "details" ? (
            <DetailsStep entry={flow.entry} onClose={close} />
          ) : (
            <UninstallStep entry={flow.entry} onConfirm={() => uninstall(flow)} onCancel={close} />
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
