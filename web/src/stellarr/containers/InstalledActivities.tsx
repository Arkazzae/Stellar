import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { Icon } from "@stellarr/icons";
import { Button } from "@stellarr/ui";
import { DialogClose } from "@/components/primitives/DialogClose";
import { Modal } from "@/components/primitives/Modal";
import { request } from "@/lib/api/transport";
import type { VoiceActivitySummary } from "@/lib/realtime/types";
import { useLanguage } from "@/state/LanguageProvider";
import { useOptionalRealtime } from "@/state/PortsProvider";
import { ActivityFrame } from "./ActivityFrame";

interface Entry {
  description: string;
  artwork?: { icon: string | null; cover: string | null };
  install: string;
  name: string;
  version: string;
  running: boolean;
  supported: boolean;
  devices: string[] | null;
  seats?: number | null;
}
interface Launch {
  install: string;
  access: string;
  src: string;
  permissions: string[];
  youtube?: boolean;
  title: string;
}
interface LaunchReply {
  install?: string;
  access: string;
  src: string;
  permissions: string[];
  youtube?: boolean;
  title: string;
}

export interface VoiceActivityBarControl {
  live: VoiceActivitySummary | null;
  joined: boolean;
  busy: boolean;
  join: () => void;
  leave: () => void;
}

interface ActivityScope {
  children?: ReactNode;
  channel?: string;
  muted?: boolean;
  cameraOn?: boolean;
  pickerOpen?: boolean;
  onClosePicker?: () => void;
  onVoiceBarChange?: (state: VoiceActivityBarControl | null) => void;
}
type Selection =
  | { install?: string; request: AbortController; session?: Launch }
  | { install: string; session: Launch; request?: undefined };

function deviceClass() {
  if (!window.matchMedia?.("(pointer: coarse)").matches) return "desktop";
  return Math.min(screen.width, screen.height) >= 600 ? "tablet" : "phone";
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return (parts[0] ?? "").slice(0, 2).toUpperCase();
  return ((parts[0]?.[0] ?? "") + (parts[1]?.[0] ?? "")).toUpperCase();
}

async function call<T>(channel: string | undefined, path: string, input: object, signal?: AbortSignal): Promise<T> {
  const response = await request("/api/activities/" + encodeURIComponent(channel ?? "personal") + "/" + path, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(input),
  }, signal);
  if (!response.ok) throw Object.assign(new Error(await response.text()), { status: response.status });
  return response.json() as Promise<T>;
}

interface LibraryProps {
  entries: Entry[];
  voice: boolean;
  busy: boolean;
  currentInstall?: string;
  onChoose: (install: string) => void;
}

function ActivityLibrary({ entries, voice, busy, currentInstall, onChoose }: LibraryProps) {
  const { t } = useLanguage();
  const libraryId = useId();
  return (
    <ul className={voice ? "activity-picker-grid" : "personal-activity-library"}>
      {entries.map(entry => {
        const isCurrent = currentInstall === entry.install;
        let label = t("stage.launchForMyself");
        if (voice) label = t(entry.running ? "stage.joinActivity" : "stage.beginActivity");
        if (isCurrent) label = t("stage.returnToActivity");
        else if (!entry.supported) label = entry.devices?.join(", ") || t("stage.unsupportedActivityDevice");
        const action = {
          disabled: busy || (!entry.supported && !isCurrent),
          "aria-label": entry.name + " · " + label,
          onClick: () => onChoose(entry.install),
        };
        if (voice) {
          const cover = entry.artwork?.cover ?? null;
          const icon = entry.artwork?.icon ?? null;
          return (
            <li key={entry.install}>
              <button
                type="button"
                className="activity-picker-card"
                aria-describedby={libraryId + entry.install}
                {...action}
              >
                <span className="ap-card-stage">
                  <span className="ap-card-banner">
                    {cover ? (
                      <img alt="" className="ap-card-cover" src={cover} loading="lazy" />
                    ) : icon ? (
                      <img alt="" aria-hidden="true" className="ap-card-cover ambient" src={icon} loading="lazy" />
                    ) : (
                      <span className="ap-card-cover fallback" aria-hidden="true">
                        <Icon name="game" size={28} />
                      </span>
                    )}
                  </span>
                  {typeof entry.seats === "number" && entry.seats > 0 && (
                    <span className="ap-card-seats">{t("stage.upToSeats", { count: entry.seats })}</span>
                  )}
                  <span className="ap-card-icon">
                    {icon ? (
                      <img alt="" src={icon} loading="lazy" />
                    ) : (
                      <span className="ap-card-initials" aria-hidden="true">{initials(entry.name)}</span>
                    )}
                  </span>
                </span>
                <span className="ap-card-copy">
                  <strong className="activity-picker-name">{entry.name}</strong>
                  <span className="activity-picker-description" id={libraryId + entry.install}>
                    {entry.description}
                  </span>
                  {!entry.supported && !isCurrent && (
                    <span className="ap-card-device">
                      <Icon name="mobile" size={14} />
                      {label}
                    </span>
                  )}
                </span>
                {!action.disabled && (
                  <span className="ap-card-veil" aria-hidden="true">
                    <span className="ap-card-start">
                      <Icon name="play" size={14} />
                      {label}
                    </span>
                    <span className="ap-card-veil-sub">{entry.name}</span>
                  </span>
                )}
              </button>
            </li>
          );
        }
        const artwork = (
          <span className="personal-activity-image">
            {entry.artwork?.cover && <img alt="" className="personal-activity-cover" src={entry.artwork.cover} loading="lazy" />}
            {entry.artwork?.icon && <img alt="" className="personal-activity-icon" src={entry.artwork.icon} loading="lazy" />}
          </span>
        );
        return (
          <li key={entry.install}>
            {artwork}
            <div className="personal-activity-details">
              <h3>{entry.name}</h3>
              <p>{entry.description}</p>
              <button type="button" className="st-chip st-button" {...action}>
                {label}
              </button>
            </div>
          </li>
        );
      })}
    </ul>
  );
}

export function InstalledActivities(props: ActivityScope) {
  return <ActivitiesForScope key={props.channel ?? "personal"} {...props} />;
}

function ActivitiesForScope({
  children,
  channel,
  muted,
  cameraOn,
  pickerOpen = false,
  onClosePicker,
  onVoiceBarChange,
}: ActivityScope) {
  const { t } = useLanguage();
  const realtime = useOptionalRealtime();
  const titleId = useId();
  const [entries, setEntries] = useState<Entry[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [reloadSeq, setReloadSeq] = useState(0);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [liveRemote, setLiveRemote] = useState<VoiceActivitySummary | null>(null);
  const [problem, setProblem] = useState<"stage.activityEnded" | "stage.activityLaunchFailed" | null>(null);
  const [showPeople, setShowPeople] = useState(false);
  const [query, setQuery] = useState("");
  const [wasOpen, setWasOpen] = useState(pickerOpen);
  if (pickerOpen !== wasOpen) {
    setWasOpen(pickerOpen);
    if (!pickerOpen && query) setQuery("");
  }

  const current = useRef<Selection | null>(null);
  const activeFrame = useRef<string | null>(null);
  const entriesRef = useRef<Entry[]>(entries);
  const liveRemoteRef = useRef<VoiceActivitySummary | null>(liveRemote);
  const onClosePickerRef = useRef(onClosePicker);

  useEffect(() => {
    entriesRef.current = entries;
    liveRemoteRef.current = liveRemote;
    onClosePickerRef.current = onClosePicker;
  }, [entries, liveRemote, onClosePicker]);

  const session = selection?.session;
  const busy = Boolean(selection?.request);
  const watchLibrary = !channel || pickerOpen || selection !== null;

  const select = useCallback((next: Selection | null) => {
    current.current?.request?.abort();
    if (current.current?.session !== next?.session) {
      activeFrame.current = null;
      setShowPeople(false);
    }
    current.current = next;
    setSelection(next);
  }, []);

  const selectSession = useCallback((next?: Launch) => {
    select(next ? { install: next.install, session: next } : null);
  }, [select]);

  useEffect(() => () => {
    current.current?.request?.abort();
    current.current = null;
  }, []);

  useEffect(() => {
    if (!channel || !realtime) return;
    return realtime.on("VoiceActivityChanged", event => {
      if (event.channelId === channel) {
        setLiveRemote(event.activity);
      }
    });
  }, [channel, realtime]);

  useEffect(() => {
    if (!watchLibrary) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let retry = 500;
    // Each query belongs to the launch/lease that triggered this effect.
    const relevant = () => !controller.signal.aborted && current.current === selection;
    function endSelection() {
      if (selection) { select(null); setProblem("stage.activityEnded"); }
    }
    async function refresh() {
      let next = 10000;
      try {
        const lease = selection?.session && { install: selection.session.install, access: selection.session.access };
        const list = await call<Entry[]>(channel, "list", { device: deviceClass(), lease }, controller.signal);
        if (!relevant()) return;
        setEntries(list);
        setLoadFailed(false);
        if (channel) {
          const runningEntry = list.find(item => item.running);
          setLiveRemote(runningEntry ? {
            install: runningEntry.install,
            name: runningEntry.name,
            icon: runningEntry.artwork?.icon ?? null,
          } : null);
        }
        if (selection?.session && !list.some(item => item.install === selection.session?.install && item.running)) {
          endSelection();
        } else if (selection?.request && selection.install && !list.some(item => item.install === selection.install)) {
          selectSession(selection.session);
          setProblem("stage.activityLaunchFailed");
        }
        retry = 500;
      } catch (error) {
        if (!relevant()) return;
        const status = error instanceof Error && "status" in error ? error.status : undefined;
        setLoadFailed(true);
        if (status === 401 || status === 403 || status === 404 || status === 410) {
          setEntries([]);
          if (channel) setLiveRemote(null);
          endSelection();
        }
        if (status === 403) {
          // The voice stage can mount before the server confirms its seat.
          next = retry;
          retry = Math.min(10000, retry * 2);
        }
      } finally {
        if (relevant()) { setLoading(false); timer = setTimeout(refresh, next); }
      }
    }
    void refresh();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [channel, reloadSeq, select, selection, selectSession, watchLibrary]);

  const action = useCallback(<T,>(path: string, input: object, signal: AbortSignal) => {
    if (!session) throw new Error("Activity closed.");
    return call<T>(channel, path, { install: session.install, access: session.access, action: input }, signal);
  }, [session, channel]);
  const room = useCallback(async (input: Record<string, unknown>, signal: AbortSignal) => {
    const value = await action<Record<string, unknown>>("room", input, signal);
    if (current.current?.session === session && !signal.aborted && input.op === "open" && typeof input.frame === "string")
      activeFrame.current = input.frame;
    return value;
  }, [action, session]);
  const tile = useCallback((input: Record<string, unknown>, signal: AbortSignal) => action<Record<string, string | null>>("tile", input, signal), [action]);
  const lost = useCallback(() => {
    if (current.current?.session !== session) return;
    select(null);
    setProblem("stage.activityEnded");
  }, [select, session]);

  const closeFrame = useCallback(() => {
    const frame = activeFrame.current;
    if (frame) void room({ op: "close", frame }, new AbortController().signal).catch(() => {});
  }, [room]);

  const leave = useCallback(() => {
    const active = current.current?.session;
    closeFrame();
    select(null);
    if (active) {
      void call<{ ok?: boolean; running?: boolean }>(
        channel,
        "leave",
        { install: active.install, access: active.access },
      ).then(res => {
        if (channel && res?.running === false) {
          setLiveRemote(null);
        }
      }).catch(() => {});
    }
  }, [channel, closeFrame, select]);

  const launch = useCallback(async (install?: string) => {
    const previousSession = current.current?.session;
    const requestCtrl = new AbortController();
    const pending: Selection = { install, request: requestCtrl, session: previousSession };
    select(pending);
    setProblem(null);
    try {
      const device = deviceClass();
      let opened: LaunchReply;
      try {
        opened = await call<LaunchReply>(
          channel,
          "launch",
          install ? { install, device } : { device },
          requestCtrl.signal,
        );
      } catch (error) {
        const status = error instanceof Error && "status" in error ? error.status : undefined;
        if (channel && install && status === 409) {
          opened = await call<LaunchReply>(channel, "launch", { device }, requestCtrl.signal);
        } else {
          throw error;
        }
      }
      const resolvedInstall = opened.install ?? install ?? liveRemoteRef.current?.install;
      if (!resolvedInstall) throw new Error("Missing activity install");
      if (current.current === pending) {
        closeFrame();
        selectSession({ ...opened, install: resolvedInstall });
        if (channel) {
          const matched = entriesRef.current.find(e => e.install === resolvedInstall);
          setLiveRemote(prev => ({
            install: resolvedInstall,
            name: opened.title,
            icon: matched?.artwork?.icon ?? (prev?.install === resolvedInstall ? prev.icon : null),
          }));
        }
        onClosePickerRef.current?.();
      }
    } catch {
      if (current.current === pending) {
        selectSession(previousSession);
        setProblem("stage.activityLaunchFailed");
      }
    }
  }, [channel, closeFrame, select, selectSession]);

  const joinLive = useCallback(() => {
    if (current.current?.session || current.current?.request) return;
    void launch();
  }, [launch]);

  const liveInstall = session?.install ?? liveRemote?.install ?? null;
  const liveName = session?.title ?? liveRemote?.name ?? null;
  const liveIcon = (liveInstall ? entries.find(e => e.install === liveInstall)?.artwork?.icon : null)
    ?? (liveRemote?.install === liveInstall ? liveRemote?.icon : null)
    ?? null;
  const liveSummary = useMemo<VoiceActivitySummary | null>(
    () => (liveInstall && liveName !== null ? { install: liveInstall, name: liveName, icon: liveIcon } : null),
    [liveIcon, liveInstall, liveName],
  );
  const joined = Boolean(session);

  useEffect(() => {
    if (!channel || !onVoiceBarChange) return;
    onVoiceBarChange({
      live: liveSummary,
      joined,
      busy,
      join: joinLive,
      leave,
    });
  }, [busy, channel, joinLive, joined, leave, liveSummary, onVoiceBarChange]);

  useEffect(() => {
    if (!channel || !onVoiceBarChange) return;
    return () => onVoiceBarChange(null);
  }, [channel, onVoiceBarChange]);

  function dismissPicker() {
    if (current.current?.request) selectSession(current.current.session);
    onClosePicker?.();
  }

  const trimmedQuery = query.trim().toLowerCase();
  const filteredEntries = useMemo(() => {
    if (!channel || !trimmedQuery) return entries;
    return entries.filter(
      item =>
        item.name.toLowerCase().includes(trimmedQuery) ||
        item.description.toLowerCase().includes(trimmedQuery),
    );
  }, [channel, entries, trimmedQuery]);

  const notice = problem && (
    <div className="ap-problem" role="alert">
      <Icon name="warning" size={16} />
      <span>{t(problem)}</span>
    </div>
  );

  if (!channel) {
    const personalLibrary = entries.length ? (
      <ActivityLibrary
        entries={entries}
        voice={false}
        busy={busy}
        currentInstall={session?.install}
        onChoose={install => install === session?.install ? dismissPicker() : void launch(install)}
      />
    ) : (
      <p role="status">
        {t(loading ? "common.loading" : loadFailed ? "stage.activityListFailed" : "stage.noPersonalActivities")}
      </p>
    );
    const personalPlayer = session && (
      <>
        <button type="button" className="st-chip st-button" onClick={leave}>{t("stage.leaveActivity")}</button>
        <ActivityFrame {...session} room={room} tile={tile} onLost={lost} muted={muted} cameraOn={cameraOn} />
      </>
    );
    return <section aria-label={t("stage.personalActivities")}>{personalPlayer ?? personalLibrary}{notice}</section>;
  }

  const countMode =
    loading && entries.length === 0
      ? "many"
      : entries.length <= 1
        ? "one"
        : entries.length === 2
          ? "pair"
          : "many";

  const voiceBody = entries.length > 0 ? (
    filteredEntries.length > 0 ? (
      <ActivityLibrary
        entries={filteredEntries}
        voice
        busy={busy}
        currentInstall={session?.install}
        onChoose={install => {
          if (install === session?.install) {
            setShowPeople(false);
            dismissPicker();
          } else void launch(install);
        }}
      />
    ) : (
      <div className="asset-empty ap-empty" role="status">
        <Icon name="search" size={22} />
        <b>{t("stage.noActivityMatch")}</b>
        <span>{t("stage.noActivityMatchDetail")}</span>
        <Button variant="secondary" size="sm" type="button" onClick={() => setQuery("")}>
          {t("stage.clearActivitySearch")}
        </Button>
      </div>
    )
  ) : loading ? (
    <div className="activity-picker-skeleton" role="status">
      <span className="ap-sr-only">{t("common.loading")}</span>
      {Array.from({ length: 3 }, (_, index) => (
        <div key={index} className="ap-skeleton-card" aria-hidden="true">
          <div className="ap-skeleton-banner" />
          <div className="ap-skeleton-icon" />
          <div className="ap-skeleton-lines">
            <b />
            <em />
          </div>
        </div>
      ))}
    </div>
  ) : loadFailed ? (
    <div className="dlg-retry" role="status">
      <span>{t("stage.activityListFailed")}</span>
      <Button
        variant="secondary"
        size="sm"
        type="button"
        onClick={() => {
          setLoading(true);
          setReloadSeq(n => n + 1);
        }}
      >
        {t("room.retry")}
      </Button>
    </div>
  ) : (
    <div className="asset-empty ap-empty" role="status">
      <Icon name="game" size={24} />
      <b>{t("stage.noVoiceActivitiesTitle")}</b>
      <span>{t("stage.noVoiceActivities")}</span>
    </div>
  );

  return (
    <>
      {session && (
        <section className="voice-activity-stage" aria-label={session.title} data-view={showPeople ? "people" : "activity"}>
          <ActivityFrame {...session} room={room} tile={tile} onLost={lost} muted={muted} cameraOn={cameraOn} hidden={showPeople}>
            <div className="activity-stage-switch" role="group" aria-label={t("stage.callView")}>
              <button type="button" aria-pressed={!showPeople} onClick={() => setShowPeople(false)}>
                <Icon name="game" size={16} /><span>{session.title}</span>
              </button>
              <button type="button" aria-pressed={showPeople} onClick={() => setShowPeople(true)}>
                <Icon name="users" size={16} /><span>{t("room.people")}</span>
              </button>
            </div>
          </ActivityFrame>
        </section>
      )}
      {!pickerOpen && notice}
      <div className="call-participants" hidden={Boolean(session) && !showPeople}>{children}</div>
      {pickerOpen && createPortal(
        <Modal open onClose={dismissPicker} labelledBy={titleId} className="sv-wrap">
          <div className="sv activity-picker" data-count={countMode}>
            <header className="sv-bar">
              <div className="sv-bar-txt">
                <h2 id={titleId}>{t("stage.voicePickerTitle")}</h2>
                <p>{t("stage.voicePickerSubtitle")}</p>
              </div>
              {entries.length >= 4 && (
                <div className="sv-bar-end">
                  <div className="dlg-search ap-search">
                    <span className="dlg-si" aria-hidden="true">
                      <Icon name="search" size={15} />
                    </span>
                    <input
                      type="search"
                      className="dlg-input"
                      value={query}
                      onChange={event => setQuery(event.target.value)}
                      placeholder={t("stage.searchActivities")}
                      aria-label={t("stage.searchActivities")}
                    />
                  </div>
                </div>
              )}
              <DialogClose label={t("common.close")} onClose={dismissPicker} />
            </header>
            <div className="sv-panes">
              {notice}
              {voiceBody}
            </div>
          </div>
        </Modal>,
        document.fullscreenElement ?? document.body,
      )}
    </>
  );
}
