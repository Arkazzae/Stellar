import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import type { RealtimeClient } from "@/lib/realtime/types";
import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { detectDevice, type Participant } from "../../../../sdk/client.js";
import { useLanguage } from "@/state/LanguageProvider";
import { bindMediaHost, type HostMediaControls, type ActivityCamera } from "../../../../sdk/host/media-host.js";

type Action = Record<string, unknown>;
function framePortrait(person: Participant, base: string): Participant {
  return person.avatar?.startsWith("/_stellarr/avatars/")
    ? { ...person, avatar: new URL(person.avatar.slice(1), base).href } : person;
}

/** Somebody in the activity as its own roster says, named so two of the same name differ. */
export interface FramePlayer {
  id: string;
  label: string;
  avatarUrl?: string;
  connected: boolean;
}

/** A device an activity may ask for. */
export type DeviceKind = "cameraTile" | "microphoneLevel";

/** The frame's own controls, for a bar that places them. */
export interface FrameParts {
  /** The activity's own volume. */
  volume: ReactNode;
  /** Where the SDK host puts what stops a device the activity uses. */
  devices: ReactNode;
  players: readonly FramePlayer[];
}

interface ActivityFrameProps {
  src: string;
  title: string;
  permissions: string[];
  youtube?: boolean;
  muted?: boolean;
  cameraOn?: boolean;
  room: (action: Action, signal: AbortSignal) => Promise<Record<string, unknown>>;
  liveRoom?: ActivityFrameProps["room"];
  realtime?: RealtimeClient | null;
  camera?: () => ActivityCamera;
  cloneLocalCamera?: () => MediaStreamTrack | undefined;
  tile?: (action: Action, signal: AbortSignal) => Promise<Record<string, string | null>>;
  onLost: () => void;
  /**
   * Lays out the volume, the device controls and who plays. Without it they
   * sit in a plain row above the frame, as the development preview has them.
   */
  bar?: (parts: FrameParts) => ReactNode;
  /**
   * Asks before the activity gets a device, with the app's own question;
   * resolves true to allow. Without it the SDK host asks in its own dialog.
   */
  ask?: (kind: DeviceKind, signal: AbortSignal) => Promise<boolean>;
}

export function ActivityFrame({ src, title, permissions, youtube, muted = false, cameraOn, room, liveRoom, realtime, camera, cloneLocalCamera, tile, onLost, bar, ask }: ActivityFrameProps) {
  const { t, locale } = useLanguage();
  const target = useMemo(() => {
    const url = new URL(src, window.location.href);
    const local = url.origin === window.location.origin;
    const nonce = local ? crypto.randomUUID() : undefined;
    if (nonce) url.hash = "stellarr=" + nonce;
    return { src: url.href, origin: local ? "null" : url.origin, nonce };
  }, [src]);
  const language = useRef(locale);
  useEffect(() => { language.current = locale; }, [locale]);
  const words = useRef(t);
  useEffect(() => { words.current = t; }, [t]);
  const asking = useRef(ask);
  useEffect(() => { asking.current = ask; }, [ask]);
  const [players, setPlayers] = useState<Participant[]>([]);
  const frame = useRef<HTMLIFrameElement>(null);
  const volume = useRef<HTMLInputElement>(null);
  const controls = useRef<HTMLSpanElement>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const media = useRef<HostMediaControls | null>(null);
  const callMuted = useRef(muted);
  useEffect(() => { callMuted.current = muted; }, [muted]);
  useEffect(() => { if (!cameraOn) media.current?.stopCamera(); }, [cameraOn]);

  useEffect(() => {
    if (!volume.current || !controls.current || !frame.current) return;
    const controller = new AbortController();
    const { origin, nonce } = target;
    // Local installations serve frame assets below the launch-specific directory.
    const portraitBase = nonce ? target.src : origin + "/";
    media.current = bindMediaHost({ frame: frame.current, origin, nonce, youtube, title, permissions, controls: controls.current,
      volume: volume.current, language: () => language.current ?? document.documentElement.lang, muted: () => callMuted.current,
      camera, cloneLocalCamera, tile: tile ? action => tile(action, controller.signal) : undefined, overlay: viewport.current ?? undefined,
      ask: asking.current ? (kind, signal) => asking.current?.(kind, signal) ?? Promise.resolve(false) : undefined,
      labels: {
        stop: { cameraTile: words.current("activities.stopCamera"), microphoneLevel: words.current("activities.stopMicrophoneLevel") },
        camera: words.current("activities.cameraImage"),
      } });
    let frameId: string | undefined;
    let connected: MessagePort | undefined;
    const offUpdate = realtime?.on("ActivityRoomUpdate", update => {
      if (update.frame === frameId) connected?.postMessage({ update });
    });
    const offStatus = realtime?.subscribeStatus(() => {
      if (realtime.getStatus() === "connected") connected?.postMessage({ update: {} });
    });
    function bind(event: MessageEvent) {
      if (event.source !== frame.current?.contentWindow || event.origin !== origin ||
          !event.ports[0]) return;
      if (nonce ? connected || event.data?.type !== "stellarr:dev-connect:v1" || event.data?.nonce !== nonce
                : event.data !== "stellarr:dev-connect:v1") return;
      connected?.close();
      connected = event.ports[0];
      const port = connected;
      let roomEvents = false;
      let rosterVersion = -1;
      let rosterSignature = "";
      const id = frameId = crypto.randomUUID();
      port.onmessage = async ({ data }) => {
        if (!data || !Number.isSafeInteger(data.id) || !data.input) return;
        try {
          const op = data.input.op;
          const request = roomEvents && liveRoom && !["open", "loadProgress", "saveProgress"].includes(op) ? liveRoom : room;
          const value = await request({ ...data.input, frame: id }, controller.signal);
          if (controller.signal.aborted || port !== connected) return;
          if (data.input.op === "open" || data.input.op === "read") {
            value.locale = language.current || document.documentElement.lang || navigator.language;
            value.device = detectDevice();
            if (typeof value.version === "number" && value.version > rosterVersion && Array.isArray(value.participants)) {
              rosterVersion = value.version;
              const signature = JSON.stringify(value.participants);
              if (signature !== rosterSignature) {
                rosterSignature = signature;
                setPlayers(value.participants);
              }
            }
          }
          if (Array.isArray(value.participants)) value.participants = value.participants.map(person => framePortrait(person, portraitBase));
          if (value.me) value.me = framePortrait(value.me as Participant, portraitBase);
          if (data.input.op === "open") {
            roomEvents = Boolean(liveRoom && realtime && value.roomHub);
            value.roomEvents = roomEvents && data.input.roomEvents === true;
            value.volume = Number(volume.current?.value ?? 100) / 100;
            value.youtube = Boolean(nonce && youtube);
          }
          port.postMessage({ id: data.id, value });
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
      // The frame's own requests stop first, and the SDK host's teardown
      // comes last, so a failure in it cannot keep a lease polling.
      controller.abort();
      offUpdate?.();
      offStatus?.();
      window.removeEventListener("message", bind);
      connected?.close();
      if (frameId) void room({ op: "disconnect", frame: frameId }, new AbortController().signal).catch(() => {});
      const host = media.current;
      media.current = null;
      try {
        host?.dispose();
      } catch {
        // Already torn down as far as it can be.
      }
    };
  }, [target, title, permissions, youtube, room, liveRoom, realtime, camera, cloneLocalCamera, tile, onLost]);

  const roster = useMemo(() => framePlayers(players), [players]);
  const volumeControl = (
    <label className="activity-volume" title={t("activities.gameAudio")}>
      <Icon name="speaker" size={16} />
      <input type="range" aria-label={t("activities.gameAudio")} min="0" max="100" defaultValue="100" ref={volume} />
    </label>
  );
  const devices = <span className="activity-devices" ref={controls} />;

  return <>
    {bar ? bar({ volume: volumeControl, devices, players: roster }) : <>
      {roster.length > 0 && <ul className="activity-players" aria-label={t("stage.activityPlayers")}>
        {roster.map(person => <li key={person.id}>
          <Avatar id={person.id} displayName={person.label} size="xs" offline={!person.connected} avatarUrl={person.avatarUrl} />
          <span>{person.label}{!person.connected && " · " + t("stage.activityDisconnected")}</span>
        </li>)}
      </ul>}
      <div className="activity-frame-controls">
        {volumeControl}
        {devices}
      </div>
    </>}
    <div className="activity-viewport" ref={viewport}>
      <iframe ref={frame} src={target.src} title={title} referrerPolicy="no-referrer"
        sandbox={target.nonce ? "allow-scripts allow-forms allow-presentation" : "allow-scripts allow-same-origin allow-forms allow-presentation"} allow="autoplay; fullscreen" />
    </div>
  </>;
}

/** The roster as the app draws it: a name that two people share carries their username. */
function framePlayers(players: readonly Participant[]): FramePlayer[] {
  const seen = new Set<string>();
  const shared = new Set<string>();
  for (const player of players) {
    if (seen.has(player.name)) shared.add(player.name);
    seen.add(player.name);
  }
  return players.map((person) => ({
    id: person.id,
    label: shared.has(person.name) && person.username ? `${person.name} (@${person.username})` : person.name,
    ...(person.avatar ? { avatarUrl: person.avatar.replace(/^\/_stellarr\/avatars\/[^/]+\//, "/assets/") } : {}),
    connected: person.connected,
  }));
}
