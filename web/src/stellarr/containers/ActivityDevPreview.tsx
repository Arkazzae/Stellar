import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { detectDevice } from "../../../../sdk/client.js";
import { ActivityFrame } from "./ActivityFrame";
import { request } from "@/lib/api/transport";
import { useLanguage } from "@/state/LanguageProvider";

type Draft = { access: string; src: string; title: string; code: string; permissions: string[] };

async function previewCall(route: string, data: object, signal: AbortSignal) {
  const response = await request("/_stellarr/dev/" + route, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(data),
  }, signal);
  if (!response.ok) throw Object.assign(new Error(await response.text()), { status: response.status });
  return response.json();
}

/** Only mounted by a test instance that has opted in to private drafts. */
export function ActivityDevPreview({ channel, muted = false, cameraOn = false }: { channel: string; muted?: boolean; cameraOn?: boolean }) {
  const { t } = useLanguage();
  const [visible, setVisible] = useState(false);
  const [code, setCode] = useState("");
  const [draft, setDraft] = useState<Draft | null>(null);
  const [problem, setProblem] = useState(false);
  const [busy, setBusy] = useState(false);
  const launch = useRef<AbortController | null>(null);

  useEffect(() => () => launch.current?.abort(), []);
  const action = useCallback((route: string, input: Record<string, unknown>, signal: AbortSignal) => {
    if (!draft) throw new Error("No active preview.");
    return previewCall(route, { channel, code: draft.code, access: draft.access, action: input }, signal);
  }, [channel, draft]);
  const room = useCallback((input: Record<string, unknown>, signal: AbortSignal) => action("room", input, signal), [action]);
  const tile = useCallback((input: Record<string, unknown>, signal: AbortSignal) => action("tile", input, signal), [action]);
  const lost = useCallback(() => { setDraft(null); setProblem(true); }, []);

  async function open(event: FormEvent) {
    event.preventDefault();
    launch.current?.abort();
    const controller = new AbortController();
    launch.current = controller;
    setBusy(true);
    setProblem(false);
    try {
      const result = await previewCall("launch", { channel, code: code.trim(), device: detectDevice() }, controller.signal);
      setDraft({ access: result.access, src: result.src, title: result.title, code: code.trim(), permissions: result.permissions ?? [] });
    } catch {
      if (!controller.signal.aborted) setProblem(true);
    } finally {
      if (!controller.signal.aborted) setBusy(false);
    }
  }

  if (!visible) return <button type="button" className="st-chip st-button" onClick={() => setVisible(true)}>
    {t("stage.devPreview")}
  </button>;
  return <section aria-label={t("stage.devPreview")}>
    <button type="button" className="st-chip st-button" onClick={() => {
      launch.current?.abort();
      setDraft(null);
      setVisible(false);
      setBusy(false);
      setProblem(false);
    }}>{t("stage.closePreview")}</button>
    {draft ? <ActivityFrame src={draft.src} title={draft.title} permissions={draft.permissions}
      room={room} tile={tile} onLost={lost} muted={muted} cameraOn={cameraOn} /> :
      <form onSubmit={open}>
        <label>{t("stage.previewCode")} <input value={code} onChange={e => setCode(e.target.value)}
          required autoComplete="off" spellCheck={false} /></label>
        <button type="submit" className="st-chip st-button" disabled={busy}>{t("stage.openPreview")}</button>
      </form>}
    {problem && <p role="alert">{t("stage.previewFailed")}</p>}
  </section>;
}
