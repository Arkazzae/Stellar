import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import type { RealtimeClient } from "@/lib/realtime/types";
import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { GIPHY_MARK } from "@/components/room/composer/GifBrowser";
import { createActivityGifs } from "@/lib/activities/gifs";
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

/** The dock over a full-screen activity, as the activity is told it (`session.shell.dock`). */
export interface DockState {
  fullscreen: boolean;
  visible: boolean;
  /** The activity holds the edge and the player lets it. */
  held: boolean;
  /** Pixels the visible dock takes from the bottom of the frame. */
  height: number;
}

/**
 * What the activity asks of the dock, and `reset` when the frame that asked is
 * gone (reloaded, reconnected or closed), which ends its hold.
 */
export type DockRequest = "hold" | "release" | "peek" | "reset";

/** The page's dock, for an activity that may hold its edge. The SDK hears it only where this is given. */
export interface FrameShell {
  /** Keep it the same object while nothing in it changes: each new one is sent to the activity. */
  dock: DockState;
  onDock: (request: DockRequest) => void;
}

/** What the frame knows, for a bar that draws the controls. */
export interface FrameParts {
  /** Every device the activity has open, pending ones included, each needing a stop in sight. */
  devices: readonly DeviceKind[];
  stopDevice: (kind: DeviceKind) => void;
  /** GIPHY's mark once the activity has used GIFs from it, drawn by the app since the frame cannot be trusted to; null before. */
  attribution: ReactNode;
  players: readonly FramePlayer[];
}

interface ActivityFrameProps {
  src: string;
  title: string;
  permissions: string[];
  embedHosts?: string[];
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
   * Draws the device stops, GIPHY's mark and who plays. Without it they sit in
   * a plain row above the frame with a volume slider of the frame's own, as the
   * development preview has them.
   */
  bar?: (parts: FrameParts) => ReactNode;
  /** Game audio for this listener, 0–1; the page that passes it draws the control. */
  volume?: number;
  /** A new number loads the activity again in a fresh frame. */
  reload?: number;
  shell?: FrameShell;
  /**
   * Asks before the activity gets a device, with the app's own question;
   * resolves true to allow. Without it the SDK host asks in its own dialog.
   */
  ask?: (kind: DeviceKind, signal: AbortSignal) => Promise<boolean>;
}

export function ActivityFrame({ src, title, permissions, embedHosts, muted = false, cameraOn, room, liveRoom, realtime, camera, cloneLocalCamera, tile, onLost, bar, ask, volume, reload = 0, shell }: ActivityFrameProps) {
  const { t, locale } = useLanguage();
  // A reload is a new frame element with a new nonce: changing only the hash
  // of a frame's address would not load it again.
  const target = useMemo(() => {
    const url = new URL(src, window.location.href);
    const local = url.origin === window.location.origin;
    const nonce = local ? crypto.randomUUID() : undefined;
    if (nonce) url.hash = "stellarr=" + nonce;
    return { src: url.href, origin: local ? "null" : url.origin, nonce, generation: reload };
  }, [src, reload]);
  const language = useRef(locale);
  useEffect(() => { language.current = locale; }, [locale]);
  const words = useRef(t);
  useEffect(() => { words.current = t; }, [t]);
  const asking = useRef(ask);
  useEffect(() => { asking.current = ask; }, [ask]);
  const [players, setPlayers] = useState<Participant[]>([]);
  const [devices, setDevices] = useState<DeviceKind[]>([]);
  const [ownVolume, setOwnVolume] = useState(100);
  const level = volume ?? ownVolume / 100;
  const levelRef = useRef(level);
  const shellRef = useRef(shell);
  useEffect(() => { shellRef.current = shell; }, [shell]);
  const portRef = useRef<MessagePort | undefined>(undefined);
  // The frame whose activity used GIFs; the mark stays until the frame changes.
  const [gifsUsedBy, setGifsUsedBy] = useState<string | null>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const media = useRef<HostMediaControls | null>(null);
  useEffect(() => {
    levelRef.current = level;
    media.current?.setVolume(level);
  }, [level]);
  const dock = shell?.dock;
  useEffect(() => {
    if (dock) portRef.current?.postMessage({ shell: { dock } });
  }, [dock]);
  const callMuted = useRef(muted);
  useEffect(() => { callMuted.current = muted; }, [muted]);
  useEffect(() => { if (!cameraOn) media.current?.stopCamera(); }, [cameraOn]);

  useEffect(() => {
    if (!frame.current) return;
    const controller = new AbortController();
    const { origin, nonce } = target;
    // Local installations serve frame assets below the launch-specific directory.
    const portraitBase = nonce ? target.src : origin + "/";
    // This page asks GIPHY for the frame, which never reaches it.
    const gifs = permissions.includes("media.gifs")
      ? createActivityGifs({ locale: () => language.current, onUse: () => setGifsUsedBy(target.src) }) : undefined;
    media.current = bindMediaHost({ frame: frame.current, origin, nonce, embedHosts, title, permissions,
      volume: levelRef.current, onDevices: kinds => setDevices(kinds),
      language: () => language.current ?? document.documentElement.lang, muted: () => callMuted.current,
      camera, cloneLocalCamera, tile: tile ? action => tile(action, controller.signal) : undefined, overlay: viewport.current ?? undefined, gifs,
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
      portRef.current = port;
      // A frame that connects again starts without the hold its last page had.
      shellRef.current?.onDock("reset");
      let roomEvents = false;
      let rosterVersion = -1;
      let rosterSignature = "";
      const id = frameId = crypto.randomUUID();
      port.onmessage = async ({ data }) => {
        if (!data || !Number.isSafeInteger(data.id) || !data.input) return;
        try {
          const op = data.input.op;
          // The dock is the page's, so this never reaches the session.
          if (op === "shell") {
            const asked = data.input.dock;
            const page = shellRef.current;
            if (!page || !["hold", "release", "peek"].includes(asked)) {
              throw Object.assign(new Error("Unknown session operation."), { status: 400 });
            }
            page.onDock(asked);
            port.postMessage({ id: data.id, value: {} });
            return;
          }
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
            value.volume = levelRef.current;
            if (shellRef.current) value.shell = { dock: shellRef.current.dock };
            if (embedHosts) value.embedHosts = nonce ? embedHosts.filter(host => "https://" + host !== location.origin) : embedHosts;
            value.hostedEmbeds = Boolean(nonce);
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
      portRef.current = undefined;
      shellRef.current?.onDock("reset");
      setDevices([]);
      if (frameId) void room({ op: "disconnect", frame: frameId }, new AbortController().signal).catch(() => {});
      const host = media.current;
      media.current = null;
      try {
        host?.dispose();
      } catch {
        // Already torn down as far as it can be.
      }
    };
  }, [target, title, permissions, embedHosts, room, liveRoom, realtime, camera, cloneLocalCamera, tile, onLost]);

  const roster = useMemo(() => framePlayers(players), [players]);
  // A stop is asked for in state, so the bar holds no reference to the host itself.
  const [stopping, setStopping] = useState<{ kind: DeviceKind } | null>(null);
  useEffect(() => {
    if (stopping !== null) media.current?.stop(stopping.kind);
  }, [stopping]);
  const stopDevice = useCallback((kind: DeviceKind) => setStopping({ kind }), []);
  const attribution = gifsUsedBy === target.src
    ? <a className="activity-giphy" href="https://giphy.com/" target="_blank" rel="noopener noreferrer">
      <img src={GIPHY_MARK} alt={t("gif.poweredBy")} width={80} height={17} />
    </a> : null;

  return <>
    {bar ? bar({ devices, stopDevice, attribution, players: roster }) : <>
      {roster.length > 0 && <ul className="activity-players" aria-label={t("stage.activityPlayers")}>
        {roster.map(person => <li key={person.id}>
          <Avatar id={person.id} displayName={person.label} size="xs" offline={!person.connected} avatarUrl={person.avatarUrl} />
          <span>{person.label}{!person.connected && " · " + t("stage.activityDisconnected")}</span>
        </li>)}
      </ul>}
      <div className="activity-frame-controls">
        <label className="activity-volume" title={t("activities.gameAudio")}>
          <Icon name="speaker" size={16} />
          <input type="range" aria-label={t("activities.gameAudio")} min="0" max="100" value={ownVolume}
            onChange={event => setOwnVolume(Number(event.target.value))} />
        </label>
        {devices.length === 0 ? null : <span className="activity-device-controls">
          {devices.map(kind => <button type="button" key={kind} onClick={() => stopDevice(kind)}>
            {t(kind === "cameraTile" ? "activities.stopCamera" : "activities.stopMicrophoneLevel")}
          </button>)}
        </span>}
        {attribution}
      </div>
    </>}
    <div className="activity-viewport" ref={viewport}>
      <iframe key={target.generation} ref={frame} src={target.src} title={title} referrerPolicy="no-referrer"
        sandbox={target.nonce ? "allow-scripts allow-forms allow-presentation allow-downloads" : "allow-scripts allow-same-origin allow-forms allow-presentation allow-downloads"} allow="autoplay; encrypted-media; fullscreen; picture-in-picture" />
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
