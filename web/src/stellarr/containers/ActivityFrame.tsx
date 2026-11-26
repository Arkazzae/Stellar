import { useEffect, useRef, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import { useLanguage } from "@/state/LanguageProvider";
import { bindMediaHost, type HostMediaControls } from "../../../../sdk/host/media-host.js";

type Action = Record<string, unknown>;
interface ActivityFrameProps {
  children?: ReactNode;
  hidden?: boolean;
  src: string;
  title: string;
  permissions: string[];
  muted?: boolean;
  cameraOn?: boolean;
  room: (action: Action, signal: AbortSignal) => Promise<Record<string, unknown>>;
  tile: (action: Action, signal: AbortSignal) => Promise<Record<string, string | null>>;
  onLost: () => void;
}

export function ActivityFrame({ children, hidden = false, src, title, permissions, muted = false, cameraOn, room, tile, onLost }: ActivityFrameProps) {
  const { t } = useLanguage();
  const frame = useRef<HTMLIFrameElement>(null);
  const volume = useRef<HTMLInputElement>(null);
  const controls = useRef<HTMLSpanElement>(null);
  const media = useRef<HostMediaControls | null>(null);
  const callMuted = useRef(muted);
  useEffect(() => { callMuted.current = muted; }, [muted]);
  useEffect(() => { if (!cameraOn) media.current?.stopCamera(); }, [cameraOn]);

  useEffect(() => {
    if (!volume.current || !controls.current || !frame.current) return;
    const controller = new AbortController();
    const origin = new URL(src).origin;
    media.current = bindMediaHost({ frame: frame.current, origin, title, permissions, controls: controls.current,
      volume: volume.current, language: document.documentElement.lang, muted: () => callMuted.current,
      tile: action => tile(action, controller.signal) });
    let frameId: string | undefined;
    let connected: MessagePort | undefined;
    function bind(event: MessageEvent) {
      if (event.source !== frame.current?.contentWindow || event.origin !== origin ||
          event.data !== "stellarr:dev-connect:v1" || !event.ports[0]) return;
      connected?.close();
      connected = event.ports[0];
      const port = connected;
      const id = frameId = crypto.randomUUID();
      port.onmessage = async ({ data }) => {
        if (!data || !Number.isSafeInteger(data.id) || !data.input) return;
        try {
          const value = await room({ ...data.input, frame: id }, controller.signal);
          if (data.input.op === "open") value.volume = Number(volume.current?.value ?? 100) / 100;
          if (!controller.signal.aborted) port.postMessage({ id: data.id, value });
        } catch (error) {
          if (controller.signal.aborted) return;
          const status = error instanceof Error && "status" in error ? error.status : 0;
          port.postMessage({ id: data.id, error: error instanceof Error ? error.message : "Activity request failed.", status });
          if (port === connected && ["open", "read"].includes(data.input.op) && [401, 403, 404, 410].some(s => s === status)) onLost();
        }
      };
    }
    window.addEventListener("message", bind);
    return () => {
      media.current?.dispose();
      media.current = null;
      if (frameId) void room({ op: "disconnect", frame: frameId }, new AbortController().signal).catch(() => {});
      controller.abort();
      connected?.close();
      window.removeEventListener("message", bind);
    };
  }, [src, title, permissions, room, tile, onLost]);

  return <>
    <div className="activity-frame-controls">
      {children}
      <label className="activity-volume" title={t("stage.gameAudio")}>
        <Icon name="speaker" size={16} />
        <input type="range" aria-label={t("stage.gameAudio")} min="0" max="100" defaultValue="100" ref={volume} />
      </label>
      <span ref={controls} />
    </div>
    <div className="activity-viewport" hidden={hidden}>
      <iframe ref={frame} src={src} title={title} referrerPolicy="no-referrer"
        sandbox="allow-scripts allow-same-origin allow-forms allow-presentation" allow="autoplay; fullscreen" />
    </div>
  </>;
}
