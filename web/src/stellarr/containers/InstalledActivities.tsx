import { useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { Icon } from "@stellarr/icons";
import { DialogClose } from "@/components/primitives/DialogClose";
import { Modal } from "@/components/primitives/Modal";
import { request } from "@/lib/api/transport";
import { useLanguage } from "@/state/LanguageProvider";
import { ActivityFrame } from "./ActivityFrame";

interface Entry { description: string; artwork?: { icon: string | null; cover: string | null }; install: string; name: string; version: string; running: boolean; supported: boolean; devices: string[] | null }
interface Launch { install: string; access: string; src: string; permissions: string[]; title: string }

interface ActivityScope {
  channel?: string;
  muted?: boolean;
  cameraOn?: boolean;
  pickerOpen?: boolean;
  onClosePicker?: () => void;
}
type Selection = { install: string; request: AbortController; session?: Launch }
  | { install: string; session: Launch; request?: undefined };

function deviceClass() {
  if (!window.matchMedia?.("(pointer: coarse)").matches) return "desktop";
  return Math.min(screen.width, screen.height) >= 600 ? "tablet" : "phone";
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
        if (voice) label = t(entry.running ? "stage.joinActivity" : "stage.startActivity");
        if (isCurrent) label = t("stage.returnToActivity");
        else if (!entry.supported) label = entry.devices?.join(", ") || t("stage.unsupportedActivityDevice");
        const artwork = (
          <span className="personal-activity-image">
            {entry.artwork?.cover && <img alt="" className="personal-activity-cover" src={entry.artwork.cover} loading="lazy" />}
            {entry.artwork?.icon && <img alt="" className="personal-activity-icon" src={entry.artwork.icon} loading="lazy" />}
          </span>
        );
        const action = {
          disabled: busy || (!entry.supported && !isCurrent),
          "aria-label": entry.name + " · " + label,
          onClick: () => onChoose(entry.install),
        };
        if (voice) return (
          <li key={entry.install}>
            <button type="button" className="activity-picker-card" aria-describedby={libraryId + entry.install} {...action}>
              {artwork}
              <strong className="activity-picker-name">{entry.name}</strong>
              <span className="activity-picker-description" id={libraryId + entry.install}>{entry.description}</span>
              <span className="activity-picker-action">
                {label}{!action.disabled && <Icon name="arrowRight" size={16} />}
              </span>
            </button>
          </li>
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

function ActivitiesForScope({ channel, muted, cameraOn, pickerOpen = false, onClosePicker }: ActivityScope) {
  const { t } = useLanguage();
  const titleId = useId();
  const [entries, setEntries] = useState<Entry[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [problem, setProblem] = useState<"stage.activityEnded" | "stage.activityLaunchFailed" | null>(null);
  const current = useRef<Selection | null>(null);
  const activeFrame = useRef<string | null>(null);
  const session = selection?.session;
  const busy = Boolean(selection?.request);
  const watchLibrary = !channel || pickerOpen || selection !== null;

  const select = useCallback((next: Selection | null) => {
    current.current?.request?.abort();
    if (current.current?.session !== next?.session) activeFrame.current = null;
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
        if (selection?.session && !list.some(item => item.install === selection.session?.install && item.running)) {
          endSelection();
        } else if (selection?.request && !list.some(item => item.install === selection.install)) {
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
  }, [channel, select, selection, selectSession, watchLibrary]);

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

  function closeFrame() {
    const frame = activeFrame.current;
    if (frame) void room({ op: "close", frame }, new AbortController().signal).catch(() => {});
  }
  function leave() {
    closeFrame();
    select(null);
  }
  async function launch(install: string) {
    const request = new AbortController();
    const pending = { install, request, session };
    select(pending);
    setProblem(null);
    try {
      const opened = await call<Omit<Launch,"install">>(channel, "launch", { install, device: deviceClass() }, request.signal);
      if (current.current === pending) {
        closeFrame();
        selectSession({ ...opened, install });
        onClosePicker?.();
      }
    } catch {
      if (current.current === pending) {
        selectSession(session);
        setProblem("stage.activityLaunchFailed");
      }
    }
  }
  function dismissPicker() {
    if (current.current?.request) selectSession(current.current.session);
    onClosePicker?.();
  }

  const notice = problem && <p role="alert">{t(problem)}</p>;
  const library = entries.length ? (
    <ActivityLibrary entries={entries} voice={Boolean(channel)} busy={busy} currentInstall={session?.install}
      onChoose={install => install === session?.install ? dismissPicker() : void launch(install)} />
  ) : <p role="status">{t(loading ? "common.loading" : loadFailed ? "stage.activityListFailed" :
    channel ? "stage.noVoiceActivities" : "stage.noPersonalActivities")}</p>;

  const player = session && <>
    <button type="button" className="st-chip st-button" onClick={leave}>{t("stage.leaveActivity")}</button>
    <ActivityFrame {...session} room={room} tile={tile} onLost={lost} muted={muted} cameraOn={cameraOn} />
  </>;

  if (!channel) return <section aria-label={t("stage.personalActivities")}>{player ?? library}{notice}</section>;
  return <>
    {(session || (problem && !pickerOpen)) && <section aria-label={t("stage.installedActivities")}>
      {player}{!pickerOpen && notice}
    </section>}
    {pickerOpen && createPortal(
      <Modal open onClose={dismissPicker} labelledBy={titleId} className="sv-wrap">
        <div className="sv activity-picker">
          <header className="sv-bar">
            <div className="sv-bar-txt"><h2 id={titleId}>{t("friends.tabActivities")}</h2></div>
            <DialogClose label={t("common.close")} onClose={dismissPicker} />
          </header>
          <div className="sv-panes">{notice}{library}</div>
        </div>
      </Modal>,
      document.fullscreenElement ?? document.body,
    )}
  </>;
}
