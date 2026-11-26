import { useCallback, useEffect, useRef, useState } from "react";

import { request } from "@/lib/api/transport";
import { useLanguage } from "@/state/LanguageProvider";
import { ActivityFrame } from "./ActivityFrame";

interface Entry { description: string; artwork?: { icon: string | null; cover: string | null }; install: string; name: string; version: string; running: boolean; supported: boolean; devices: string[] | null }
interface Launch { install: string; access: string; src: string; permissions: string[]; title: string }

interface ActivityScope { channel?: string; muted?: boolean; cameraOn?: boolean }
type Selection = { install: string; request: AbortController; session?: undefined }
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

export function InstalledActivities(props: ActivityScope) {
  return <ActivitiesForScope key={props.channel ?? "personal"} {...props} />;
}

function ActivitiesForScope({ channel, muted, cameraOn }: ActivityScope) {
  const { t } = useLanguage();
  const [entries, setEntries] = useState<Entry[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [problem, setProblem] = useState(false);
  const current = useRef<Selection | null>(null);
  const activeFrame = useRef<string | null>(null);
  const session = selection?.session;
  const busy = Boolean(selection?.request);

  const select = useCallback((next: Selection | null) => {
    current.current?.request?.abort();
    current.current = next;
    activeFrame.current = null;
    setSelection(next);
  }, []);

  useEffect(() => () => {
    current.current?.request?.abort();
    current.current = null;
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let retry = 500;
    // Each query belongs to the launch/lease that triggered this effect.
    const relevant = () => !controller.signal.aborted && current.current === selection;
    function endSelection() {
      if (selection) { select(null); setProblem(true); }
    }
    async function refresh() {
      let next = 10000;
      try {
        const lease = selection?.session && { install: selection.install, access: selection.session.access };
        const list = await call<Entry[]>(channel, "list", { device: deviceClass(), lease }, controller.signal);
        if (!relevant()) return;
        setEntries(list);
        setLoadFailed(false);
        const entry = list.find(item => item.install === selection?.install);
        if (!entry || (selection?.session && !entry.running)) endSelection();
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
  }, [channel, select, selection]);

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
    setProblem(true);
  }, [select, session]);

  function leave() {
    const frame = activeFrame.current;
    if (frame) void room({ op: "close", frame }, new AbortController().signal).catch(() => {});
    select(null);
  }
  async function launch(install: string) {
    const request = new AbortController();
    const pending = { install, request };
    select(pending);
    setProblem(false);
    try {
      const opened = await call<Omit<Launch,"install">>(channel, "launch", { install, device: deviceClass() }, request.signal);
      if (current.current === pending) select({ install, session: { ...opened, install } });
    } catch {
      if (current.current === pending) { select(null); setProblem(true); }
    }
  }
  if (!entries.length && !session && !problem) {
    if (channel) return null;
    return <p role="status">{t(loading ? "common.loading" : loadFailed ? "stage.activityListFailed" : "stage.noPersonalActivities")}</p>;
  }
  if (!channel && !session) return <section aria-label={t("stage.personalActivities")}>
    <ul className="personal-activity-library">{entries.map(entry => <li key={entry.install}>
      <div className="personal-activity-image">
        {entry.artwork?.cover && <img alt="" className="personal-activity-cover" src={entry.artwork.cover} loading="lazy" />}
        {entry.artwork?.icon && <img alt="" className="personal-activity-icon" src={entry.artwork.icon} loading="lazy" />}
      </div>
      <div className="personal-activity-details">
        <h3>{entry.name}</h3>
        <p>{entry.description}</p>
        <button type="button" className="st-chip st-button" disabled={busy || !entry.supported}
          aria-label={entry.name + " · " + t("stage.launchForMyself")} onClick={() => void launch(entry.install)}>
          {entry.supported ? t("stage.launchForMyself") : entry.devices?.join(", ")}
        </button>
      </div>
    </li>)}</ul>
    {problem && <p role="alert">{t("stage.activityEnded")}</p>}
  </section>;
  return <section aria-label={t(channel ? "stage.installedActivities" : "stage.personalActivities")}>
    {session ? <>
      <button type="button" className="st-chip st-button" onClick={leave}>{t("stage.leaveActivity")}</button>
      <ActivityFrame {...session} room={room} tile={tile} onLost={lost} muted={muted} cameraOn={cameraOn} />
    </> : entries.map(entry => <button type="button" className="st-chip st-button" key={entry.install} disabled={busy || !entry.supported}
      title={entry.description} onClick={() => void launch(entry.install)}>
      {entry.artwork?.icon && <img alt="" src={entry.artwork.icon} width="22" height="22" loading="lazy" />}
      {entry.name} · {entry.supported ? t(entry.running ? "stage.joinActivity" : "stage.startActivity") : entry.devices?.join(", ")}
    </button>)}
    {problem && <p role="alert">{t("stage.activityEnded")}</p>}
  </section>;
}
