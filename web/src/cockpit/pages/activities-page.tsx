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
import { formatDayTime, formatMediumDate, formatShortDateTime, formatTime } from "../i18n/format";
import { useCockpitResource } from "../lib/use-cockpit-resource";
import { ActivityIcon, AddStep, DetailsStep, ReviewStep, UninstallStep } from "./activities/activity-dialog";
import {
  activityApi,
  explain,
  permissionShort,
  repositoryName,
  versionSince,
  type ActivityRelease,
  type InstalledActivity,
  type UpdateCheck,
} from "./activities/library";

/** What the dialog shows. Its content stays while the dialog fades out, so the step is kept apart from `open`. */
type Flow =
  | { step: "add" }
  | { step: "review"; release: ActivityRelease; current: InstalledActivity | null }
  | { step: "details"; entry: InstalledActivity }
  | { step: "uninstall"; entry: InstalledActivity };

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

/** When a look was made: the time alone today, the day and time before. */
function checkedWhen(at: string): string {
  return new Date(at).toDateString() === new Date().toDateString()
    ? t("activities.update.checkedAt", { time: formatTime(at) })
    : t("activities.update.checkedOn", { date: formatDayTime(at) });
}

function CheckedAt({ at }: { at: string }) {
  return (
    <p className="text-xs text-muted-foreground" title={formatShortDateTime(at)}>
      {checkedWhen(at)}
    </p>
  );
}

interface UpdateCellProps {
  check: UpdateCheck | null;
  /** A look started from this page is under way. */
  checking: boolean;
  /** The newer release is being read for its review. */
  opening: boolean;
  onReview: () => void;
}

/** What the server's last look at the repository found, kept since, or the look under way. */
function UpdateCell({ check, checking, opening, onReview }: UpdateCellProps) {
  if (checking) {
    return (
      <span role="status" className="flex items-center gap-2 text-muted-foreground">
        <Spinner />
        {t("activities.update.checking")}
      </span>
    );
  }
  if (!check) return <span className="text-muted-foreground">{t("activities.update.unchecked")}</span>;
  if (check.error) {
    return (
      <>
        <p className="text-danger">{t("activities.update.failed")}</p>
        <p
          className={cx(fit.narrow, "truncate text-xs text-muted-foreground")}
          title={`${check.error}\n${checkedWhen(check.checkedAt)}`}
        >
          {check.error}
        </p>
      </>
    );
  }
  if (check.version) {
    return (
      <div className="flex items-center gap-3">
        <div>
          <Badge variant="warning" className="tabular-nums">
            {t("activities.update.available", { version: check.version })}
          </Badge>
          <CheckedAt at={check.checkedAt} />
        </div>
        <Button size="sm" variant="secondary" disabled={opening} onClick={onReview}>
          {opening ? <Spinner /> : null}
          {t("activities.update")}
        </Button>
      </div>
    );
  }
  return (
    <>
      <p>{t("activities.update.current")}</p>
      <CheckedAt at={check.checkedAt} />
    </>
  );
}

/** The installed version, and the day it came. */
function VersionCell({ entry }: { entry: InstalledActivity }) {
  const since = versionSince(entry);
  return (
    <>
      <p className="tabular-nums">{entry.manifest.version}</p>
      {since ? (
        <p
          className="text-xs whitespace-nowrap text-muted-foreground"
          title={t(since.updated ? "activities.updatedOn" : "activities.installedOn", {
            date: formatShortDateTime(since.at),
          })}
        >
          {formatMediumDate(since.at)}
        </p>
      ) : null}
    </>
  );
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
  checking: boolean;
  opening: boolean;
  toggling: boolean;
  onToggle: (enabled: boolean) => void;
  onCheck: () => void;
  onReview: () => void;
  onDetails: () => void;
  onUninstall: () => void;
}

function ActivityRow(props: ActivityRowProps) {
  const { entry, checking, opening, toggling, onToggle, onCheck, onReview, onDetails, onUninstall } = props;
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
      <TableCell>
        <VersionCell entry={entry} />
      </TableCell>
      <TableCell>
        <UpdateCell check={entry.check} checking={checking} opening={opening} onReview={onReview} />
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
            <DropdownMenuItem disabled={checking} onSelect={onCheck}>
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
  const [checking, setChecking] = useState<ReadonlySet<string>>(new Set());
  const [opening, setOpening] = useState<string | null>(null);
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
  const marked = (install: string, on: boolean) =>
    setChecking((current) => {
      const next = new Set(current);
      if (on) next.add(install);
      else next.delete(install);
      return next;
    });

  /**
   * Has the server look for a newer release of one installation now; it keeps
   * what it finds, a failed look included, and the row shows it. Resolves to
   * whether looking at the next one makes sense: GitHub's request limit, a
   * busy installer and a server out of reach would fail it too.
   */
  async function check(entry: InstalledActivity): Promise<boolean> {
    marked(entry.install, true);
    try {
      await activityApi.check(entry.install);
      return true;
    } catch (cause) {
      setNotice({ message: explain(cause), error: true });
      return false;
    } finally {
      await refresh();
      marked(entry.install, false);
    }
  }

  /** Reads the newer release the last look found, for its review beside what is installed. */
  async function review(entry: InstalledActivity) {
    setOpening(entry.install);
    try {
      const result = await activityApi.review(entry.source, entry.manifest.id, entry.install);
      if ("current" in result) {
        // The release the look found is gone; looking again replaces the finding.
        setNotice({ message: t("activities.upToDate", { name: entry.manifest.name }), error: false });
        await check(entry);
      } else {
        show({ step: "review", release: result, current: entry });
      }
    } catch (cause) {
      setNotice({ message: explain(cause), error: true });
    } finally {
      setOpening(null);
    }
  }

  /** One repository at a time: the server reads two releases at once at most, and GitHub limits requests. */
  async function checkAll() {
    setNotice(null);
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
            <span className="text-sm text-muted-foreground">
              {t("activities.autoCheck", { hours: data.checkHours })}
            </span>
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
                    checking={checking.has(entry.install)}
                    opening={opening === entry.install}
                    toggling={toggling === entry.install}
                    onToggle={(enabled) => void toggle(entry, enabled)}
                    onCheck={() => {
                      setNotice(null);
                      void check(entry);
                    }}
                    onReview={() => void review(entry)}
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
