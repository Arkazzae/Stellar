import { useCallback, useEffect, useRef, useState } from "react";

import { request } from "@/lib/api/transport";
import { useLanguage } from "@/state/LanguageProvider";
import { ActivityFrame } from "./ActivityFrame";

interface Entry { description: string; artwork?: { icon: string | null; cover: string | null }; install: string; name: string; version: string; running: boolean; supported: boolean; devices: string[] | null }
interface Launch { install: string; access: string; src: string; permissions: string[]; title: string }

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

export function InstalledActivities({ channel, muted, cameraOn }: { channel?: string; muted?: boolean; cameraOn?: boolean }) {
  const { t } = useLanguage();
  const [entries, setEntries] = useState<Entry[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [session, setSession] = useState<Launch | null>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState(false);
  const activeFrame = useRef<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let retry = 500;
    async function refresh() {
      let next = 10000;
      try {
        const list = await call<Entry[]>(channel, "list", { device: deviceClass() }, controller.signal);
        if (!controller.signal.aborted) { setEntries(list); setLoadFailed(false); }
        retry = 500;
      } catch (error) {
        if (!controller.signal.aborted) setLoadFailed(true);
        if (error instanceof Error && "status" in error && error.status === 403) {
          // The voice stage can mount before the server confirms its seat.
          next = retry;
          retry = Math.min(10000, retry * 2);
        }
      }
      if (!controller.signal.aborted) { setLoading(false); timer = setTimeout(refresh, next); }
    }
    void refresh();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [channel]);

  const action = useCallback(<T,>(path: string, input: object, signal: AbortSignal) => {
    if (!session) throw new Error("Activity closed.");
    return call<T>(channel, path, { install: session.install, access: session.access, action: input }, signal);
  }, [session, channel]);
  const room = useCallback(async (input: Record<string, unknown>, signal: AbortSignal) => {
    const value = await action<Record<string, unknown>>("room", input, signal);
    if (input.op === "open" && typeof input.frame === "string") activeFrame.current = input.frame;
    return value;
  }, [action]);
  const tile = useCallback((input: Record<string, unknown>, signal: AbortSignal) => action<Record<string, string | null>>("tile", input, signal), [action]);
  const lost = useCallback(() => { setSession(null); setProblem(true); }, []);

  function leave() {
    const frame = activeFrame.current;
    activeFrame.current = null;
    if (frame) void room({ op: "close", frame }, new AbortController().signal).catch(() => {});
    setSession(null);
  }
  async function launch(install: string) {
    setBusy(true); setProblem(false);
    try {
      const opened = await call<Omit<Launch,"install">>(channel, "launch", { install, device: deviceClass() });
      setSession({ ...opened, install });
    } catch { setProblem(true); }
    finally { setBusy(false); }
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
