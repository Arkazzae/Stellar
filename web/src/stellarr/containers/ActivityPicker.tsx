import { useEffect, useId, useState, type CSSProperties, type ReactNode } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { ActivityIcon, toneOf } from "@/components/activities/ActivityIcon";
import { Avatar } from "@/components/primitives/Avatar";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { Modal } from "@/components/primitives/Modal";
import { DialogHeader } from "@/components/settings/DialogHeader";
import {
  activitiesApi,
  ActivityRequestError,
  deviceClass,
  isFull,
  PERSONAL,
  type ActivityEntry,
  type ActivityLibrary,
  type ActivityOwner,
  type ActivitySummary,
} from "@/lib/activities/api";
import { refusalText, UNSUPPORTED_ON } from "@/lib/activities/reasons";
import { initials, pluralForm } from "@/lib/format/format";
import {
  useActivities,
  useDirectory,
  useFriends,
  useLanguage,
  useSession,
  useUi,
  type PickerRequest,
} from "@/state/hooks";

/** Cockpit's page for installing activities; outside this app's router. */
const COCKPIT_ACTIVITIES = "/cockpit/activities";
/** From this many entries the list is worth searching. */
const SEARCH_FROM = 4;
/** Retries after a failed list: 3, 6, 12 s and on, never more than 30 s apart. */
const retryDelayMs = (failures: number) => Math.min(3000 * 2 ** failures, 30_000);

interface Failure {
  status: number;
  /** How many tries in a row have failed. */
  count: number;
  retryAt: number;
}

/**
 * The one place an activity is picked: for the channel you are in, for
 * yourself, or, for the host, in place of the one that runs.
 *
 * Every card says what it will do before it is touched; a card that cannot
 * run here says why instead of a button that refuses.
 */
export function ActivityPicker() {
  const { picker, closePicker } = useActivities();
  const titleId = useId();
  const [shown, setShown] = useState<PickerRequest | null>(picker);
  if (picker !== null && picker !== shown) setShown(picker);

  if (shown === null) return null;
  return (
    <Modal open={picker !== null} onClose={closePicker} labelledBy={titleId} className="act-modal act-picker" exitMs={200}>
      <PickerCard request={shown} titleId={titleId} onClose={closePicker} />
    </Modal>
  );
}

function PickerCard({ request, titleId, onClose }: { request: PickerRequest; titleId: string; onClose: () => void }) {
  const { t, locale } = useLanguage();
  const activities = useActivities();
  const ui = useUi();
  const friends = useFriends();
  const directory = useDirectory();
  const { connection, user } = useSession();
  const confirm = useConfirm();

  const online = connection === "connected";
  const voice = request.scope === "voice";
  const place = request.scope === "voice" ? request.channelId : PERSONAL;
  const [device] = useState(deviceClass);
  const selfId = user?.id ?? "";

  const [library, setLibrary] = useState<ActivityLibrary | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState<string | null>(null);
  const [refused, setRefused] = useState<Readonly<Record<string, string>>>({});

  useEffect(() => {
    if (!online) return;
    const controller = new AbortController();
    activitiesApi.list(place, { device }, controller.signal).then(
      (next) => {
        if (controller.signal.aborted) return;
        setLibrary(next);
        setFailure(null);
      },
      (cause: unknown) => {
        if (controller.signal.aborted) return;
        const status = cause instanceof ActivityRequestError ? cause.status : 0;
        setFailure((previous) => {
          const count = (previous?.count ?? 0) + 1;
          return { status, count, retryAt: Date.now() + retryDelayMs(count - 1) };
        });
      },
    );
    return () => controller.abort();
  }, [place, device, online, attempt]);

  // Tries again on its own, saying when, until the server answers.
  useEffect(() => {
    if (failure === null || !online) return;
    const tick = setInterval(() => setNow(Date.now()), 1000);
    const retry = setTimeout(() => setAttempt((count) => count + 1), Math.max(0, failure.retryAt - Date.now()));
    return () => {
      clearInterval(tick);
      clearTimeout(retry);
    };
  }, [failure, online]);

  const nameOf = (id: string) =>
    id === selfId ? (user?.displayName ?? "") : (directory.members.find((member) => member.id === id)?.displayName ?? "");
  const list = (names: string[]) => new Intl.ListFormat(locale, { type: "conjunction" }).format(names);

  const live: ActivitySummary | null = voice
    ? (activities.live.get(place) ?? library?.live ?? null)
    : null;
  const channel = voice ? directory.channelName(place) : "";
  const running = voice ? live : null;
  const mine = voice ? activities.voice : activities.personal;
  const inert = !online || failure !== null;

  const entries = library?.activities ?? [];
  const needle = query.trim().toLocaleLowerCase(locale);
  const shown =
    needle === ""
      ? entries
      : entries.filter((entry) => `${entry.name} ${entry.description}`.toLocaleLowerCase(locale).includes(needle));

  const start = async (entry: ActivityEntry, replace: boolean) => {
    setPending(entry.install);
    setRefused((current) => {
      const rest = { ...current };
      delete rest[entry.install];
      return rest;
    });
    try {
      await activities.launch(
        place,
        { install: entry.install, name: entry.name, icon: entry.icon, seats: entry.seats },
        replace ? { replace: true } : {},
      );
    } catch (cause) {
      setRefused((current) => ({ ...current, [entry.install]: refusalText(t, cause, device) }));
    } finally {
      setPending(null);
    }
  };

  const choose = async (entry: ActivityEntry) => {
    if (pending !== null || inert) return;
    if (!voice && mine?.install === entry.install) {
      ui.openActivity();
      onClose();
      return;
    }
    if (voice && request.change && running !== null) {
      if (running.install === entry.install) {
        onClose();
        return;
      }
      const others = running.participants.filter((id) => id !== selfId).map(nameOf).filter(Boolean);
      const swap = await confirm({
        title: t("activities.swapTitle", { current: running.name, next: entry.name }),
        description: t("activities.swapDetail", { current: running.name, channel, next: entry.name }),
        subject: {
          kind: "item",
          name: running.name,
          detail: others.length > 0 ? t("activities.insideNames", { names: list(others) }) : t("activities.insideOnlyYou"),
          mark: running.icon === null ? { icon: "game" } : { image: running.icon },
        },
        confirmLabel: t("activities.swap"),
        cancelLabel: t("common.cancel"),
      });
      if (swap) await start(entry, true);
      return;
    }
    await start(entry, false);
  };

  const actionOf = (entry: ActivityEntry): string => {
    if (pending === entry.install) return t("activities.launching");
    if (!voice && mine?.install === entry.install) return t("activities.back");
    if (voice && request.change && running?.install === entry.install) return t("activities.back");
    return voice && request.change ? t("activities.swap") : t("activities.launch");
  };

  let top = null;
  if (voice && running !== null && request.change) {
    const others = running.participants.filter((id) => id !== selfId).map(nameOf).filter(Boolean);
    top = (
      <TopRow
        caption={t("activities.runningHere")}
        summary={running}
        detail={others.length > 0 ? t("activities.youHostWith", { names: list(others) }) : t("activities.youHost")}
      >
        <Button type="button" variant="secondary" size="sm" onClick={onClose}>
          {t("activities.back")}
        </Button>
      </TopRow>
    );
  } else if (voice && running !== null && activities.voice?.place !== place) {
    const full = isFull(running);
    top = (
      <TopRow
        caption={t("activities.runningHere")}
        summary={running}
        detail={t("activities.inside", { count: running.participants.length })}
      >
        {full ? (
          <span className="act-full" title={t("activities.fullDetail", { count: running.participants.length, seats: running.seats ?? 0 })}>
            {t("activities.full")}
          </span>
        ) : (
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={() => {
              activities.join(place);
              onClose();
            }}
          >
            {t("activities.join")}
          </Button>
        )}
      </TopRow>
    );
  } else if (!voice && activities.personal !== null) {
    const session = activities.personal;
    top = (
      <TopRow
        summary={{ install: session.install, name: session.title, icon: session.icon }}
        detail={t("activities.runningBackground")}
      >
        <Button type="button" variant="secondary" size="sm" onClick={() => activities.leave("personal")}>
          {t("activities.close")}
        </Button>
        <Button
          type="button"
          variant="primary"
          size="sm"
          onClick={() => {
            ui.openActivity();
            onClose();
          }}
        >
          {t("activities.back")}
        </Button>
      </TopRow>
    );
  }

  let body;
  if (library === null && online && failure === null) {
    body = <Skeleton label={t("common.loading")} />;
  } else if (library !== null && entries.length === 0) {
    body = <EmptyState library={library} onMessage={(owner) => {
      friends.messagePerson(owner.id);
      onClose();
    }} />;
  } else if (library !== null) {
    body = (
      <>
        {entries.length >= SEARCH_FROM ? (
          <div className="act-find">
            <div className="dlg-search">
              <span className="dlg-si" aria-hidden="true">
                <Icon name="search" size={14} />
              </span>
              <input
                className="dlg-input"
                type="search"
                aria-label={t("activities.search")}
                placeholder={t("activities.search")}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
            </div>
            <span className="act-count" aria-live="polite">
              {t(`activities.count.${pluralForm(locale, shown.length)}`, { count: shown.length })}
            </span>
          </div>
        ) : null}
        {shown.length === 0 ? (
          <div className="asset-empty" role="status">
            <Icon name="search" size={22} />
            <b>{t("activities.noMatch", { query: query.trim() })}</b>
            <Button type="button" variant="secondary" size="sm" onClick={() => setQuery("")}>
              {t("activities.clearSearch")}
            </Button>
          </div>
        ) : (
          <ul className="act-cards" inert={inert || undefined}>
            {shown.map((entry) => (
              <li key={entry.install}>
                <ActivityCard
                  entry={entry}
                  action={actionOf(entry)}
                  unsupported={entry.supported ? null : t(UNSUPPORTED_ON[device])}
                  reason={refused[entry.install] ?? null}
                  busy={pending === entry.install}
                  live={voice && running?.install === entry.install ? running : null}
                  onChoose={() => void choose(entry)}
                />
              </li>
            ))}
          </ul>
        )}
        {!voice && library.voiceOnly.length > 0 ? (
          <p className="act-note">
            {t(`activities.voiceOnly.${pluralForm(locale, library.voiceOnly.length)}`, {
              names: list(library.voiceOnly),
            })}
          </p>
        ) : null}
      </>
    );
  }

  return (
    <div className="am-card">
      <DialogHeader
        titleId={titleId}
        title={t("activities.title")}
        detail={voice ? t("activities.pickerVoice", { channel }) : t("activities.pickerPersonal")}
        onClose={onClose}
        centered
      />
      <div className="dlg-body act-body" data-stale={inert && library !== null ? "" : undefined}>
        {!online ? (
          <Banner title={t("activities.reconnecting")} detail={t("activities.reconnectingDetail")} />
        ) : failure !== null ? (
          <Banner
            title={t("activities.serverDown")}
            detail={
              failure.status === 0
                ? t("activities.retryInOffline", { seconds: Math.max(1, Math.ceil((failure.retryAt - now) / 1000)) })
                : t("activities.retryIn", {
                    seconds: Math.max(1, Math.ceil((failure.retryAt - now) / 1000)),
                    status: failure.status,
                  })
            }
            action={
              <Button type="button" variant="secondary" size="sm" onClick={() => setAttempt((count) => count + 1)}>
                {t("activities.retryNow")}
              </Button>
            }
          />
        ) : null}
        {top}
        {body}
      </div>
    </div>
  );
}

function TopRow({
  caption,
  summary,
  detail,
  children,
}: {
  caption?: string;
  summary: Pick<ActivitySummary, "install" | "name" | "icon">;
  detail: string;
  children: ReactNode;
}) {
  return (
    <section className="act-top">
      {caption === undefined ? null : <h3 className="fld-lab">{caption}</h3>}
      <div className="act-top-row">
        <ActivityIcon install={summary.install} name={summary.name} icon={summary.icon} size={36} />
        <span className="dlg-who">
          <b>{summary.name}</b>
          <span>{detail}</span>
        </span>
        <span className="act-top-acts">{children}</span>
      </div>
    </section>
  );
}

function Banner({ title, detail, action }: { title: string; detail: string; action?: ReactNode }) {
  return (
    <div className="act-banner" role="status">
      <Icon name={action === undefined ? "refresh" : "warning"} size={16} />
      <span className="act-banner-text">
        <b>{title}</b>
        <span>{detail}</span>
      </span>
      {action}
    </div>
  );
}

function Skeleton({ label }: { label: string }) {
  return (
    <div className="act-cards act-skeleton" role="status" aria-live="polite">
      <span className="sr-only">{label}</span>
      {[0, 1, 2].map((card) => (
        <div className="act-card" key={card} aria-hidden="true">
          <span className="act-card-art" />
          <span className="act-card-icon" />
          <span className="act-skeleton-lines">
            <b />
            <em />
          </span>
        </div>
      ))}
    </div>
  );
}

interface CardProps {
  entry: ActivityEntry;
  action: string;
  /** Why it cannot run on this device, in words; null when it can. */
  unsupported: string | null;
  /** Why the server refused it a moment ago. */
  reason: string | null;
  busy: boolean;
  /** What runs of it on the channel, for the one that does. */
  live: ActivitySummary | null;
  onChoose: () => void;
}

/** One activity: the whole card is the button, and it always says what it does. */
function ActivityCard({ entry, action, unsupported, reason, busy, live, onChoose }: CardProps) {
  const { t, locale } = useLanguage();
  const id = useId();
  const facts = [
    entry.seats === null ? null : t(`activities.seats.${pluralForm(locale, entry.seats)}`, { count: entry.seats }),
    entry.permissions.includes("media.cameraTile") ? t("activities.factCamera") : null,
    entry.permissions.includes("media.microphoneLevel") ? t("activities.factMicrophoneLevel") : null,
    entry.permissions.includes("media.gifs") ? t("activities.factGifs") : null,
  ].filter((fact): fact is string => fact !== null);
  const art = entry.cover ?? entry.icon;

  return (
    <button
      type="button"
      className="act-card"
      data-unsupported={unsupported === null ? undefined : ""}
      data-live={live === null ? undefined : ""}
      aria-disabled={unsupported !== null || undefined}
      aria-busy={busy || undefined}
      aria-labelledby={`${id}-name ${id}-action`}
      aria-describedby={`${id}-description${reason === null ? "" : ` ${id}-reason`}`}
      onClick={unsupported === null ? onChoose : undefined}
    >
      <span className="act-card-art" style={{ "--act-hue": toneOf(entry.install) } as CSSProperties}>
        {art === null ? (
          <span className="act-card-initials" aria-hidden="true">
            {initials(entry.name)}
          </span>
        ) : (
          <img src={art} alt="" loading="lazy" />
        )}
      </span>
      <ActivityIcon className="act-card-icon" install={entry.install} name={entry.name} icon={entry.icon} size={22} />
      <b className="act-card-name" id={`${id}-name`}>
        {entry.name}
      </b>
      <span className="act-card-description" id={`${id}-description`}>
        {entry.description}
      </span>
      {facts.length > 0 || live !== null ? (
        <span className="act-card-facts">
          {live === null ? null : (
            <span className="act-fact live">{t("activities.inside", { count: live.participants.length })}</span>
          )}
          {facts.map((fact) => (
            <span className="act-fact" key={fact}>
              {fact}
            </span>
          ))}
        </span>
      ) : null}
      {reason === null ? null : (
        <span className="act-card-reason" id={`${id}-reason`} role="alert">
          <Icon name="warning" size={14} />
          {reason}
        </span>
      )}
      <span className="act-card-action" id={`${id}-action`}>
        {unsupported ?? action}
      </span>
    </button>
  );
}

function OwnerRow({ owner, onMessage }: { owner: ActivityOwner; onMessage: (owner: ActivityOwner) => void }) {
  const { t } = useLanguage();
  return (
    <div className="dlg-row act-owner">
      <Avatar id={owner.id} displayName={owner.name} avatarUrl={owner.avatar ?? undefined} size="md" />
      <span className="dlg-who">
        <b>{owner.name}</b>
        <span>{t("activities.owner")}</span>
      </span>
      <Button type="button" variant="secondary" size="sm" onClick={() => onMessage(owner)}>
        {t("activities.message")}
      </Button>
    </div>
  );
}

function EmptyState({ library, onMessage }: { library: ActivityLibrary; onMessage: (owner: ActivityOwner) => void }) {
  const { t } = useLanguage();
  return (
    <div className="act-state">
      <Icon name="game" size={26} />
      <h3>{t("activities.emptyTitle")}</h3>
      {library.viewerIsOwner ? (
        <>
          <p>{t("activities.emptyOwner")}</p>
          <a className="act-link" href={COCKPIT_ACTIVITIES}>
            {t("activities.openCockpit")}
          </a>
        </>
      ) : (
        <>
          <p>{t("activities.emptyOther")}</p>
          {library.owner === null ? null : <OwnerRow owner={library.owner} onMessage={onMessage} />}
        </>
      )}
    </div>
  );
}
