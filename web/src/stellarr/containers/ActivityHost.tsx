import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { Icon } from "@stellarr/icons";
import { ActivityIcon } from "@/components/activities/ActivityIcon";
import { ConfirmDialog, useConfirm } from "@/components/primitives/ConfirmDialog";
import { Popover } from "@/components/primitives/Popover";
import { ActivityCameraTransport } from "@/lib/activities/camera";
import { activitiesApi } from "@/lib/activities/api";
import { dockAlways, setDockAlways, useDockAlways } from "@/lib/activities/dockPreference";
import {
  useActivities,
  useDirectory,
  useLanguage,
  usePorts,
  useSession,
  useUi,
  useVoiceCall,
  useVoiceLive,
  type ActivitySession,
} from "@/state/hooks";
import { useOptionalRealtime } from "@/state/PortsProvider";
import type { SessionSlot } from "@/state/ActivitiesProvider";

import {
  ActivityFrame,
  type DeviceKind,
  type DockRequest,
  type DockState,
  type FrameParts,
  type FrameShell,
} from "./ActivityFrame";

/** How long the dock shows by itself: on entering full screen, and when the activity asks. */
const DOCK_PEEK_MS = 2500;
/** How long the dock stays after the pointer leaves it. */
const DOCK_LINGER_MS = 400;
/** How often an activity may ask to show the dock. */
const DOCK_PEEK_GAP_MS = 10_000;

/** A screen with no pointer to reach the edge: there the dock stays up unless the activity holds it. */
const NO_HOVER = "(hover: none)";
function subscribeHover(listener: () => void): () => void {
  const query = typeof window.matchMedia === "function" ? window.matchMedia(NO_HOVER) : null;
  query?.addEventListener?.("change", listener);
  return () => query?.removeEventListener?.("change", listener);
}
const touchOnly = () => typeof window.matchMedia === "function" && window.matchMedia(NO_HOVER).matches;

/** The devices an activity may hold, in the order the panel lists them. */
const DEVICES: readonly DeviceKind[] = ["cameraTile", "microphoneLevel"];
const DEVICE = {
  cameraTile: { name: "activities.device.cameraTile", stop: "activities.stopCamera", icon: "camera" },
  microphoneLevel: { name: "activities.device.microphoneLevel", stop: "activities.stopMicrophoneLevel", icon: "wave" },
} as const;

/** The app's question for each device an activity may ask for. */
const CONSENT = {
  cameraTile: {
    title: "activities.consent.cameraTitle",
    description: "activities.consent.cameraDetail",
    subject: "activities.consent.cameraSubject",
    allow: "activities.consent.cameraAllow",
    deny: "activities.consent.cameraDeny",
  },
  microphoneLevel: {
    title: "activities.consent.levelTitle",
    description: "activities.consent.levelDetail",
    subject: "activities.consent.levelSubject",
    allow: "activities.consent.levelAllow",
    deny: "activities.consent.levelDeny",
  },
} as const;

interface Consent {
  id: number;
  kind: DeviceKind;
  answer: (allowed: boolean) => void;
}

/**
 * Keeps `node` over `target`, wherever the layout moves it, and parks it out
 * of sight while there is no target or the target has no size: a view that
 * is not showing. The panel is never moved in the document, since a moved
 * frame reloads; it is placed over the view instead. Returns the cleanup.
 */
function cover(node: HTMLElement | null, target: HTMLElement | null): (() => void) | undefined {
  if (node === null) return;
  // Parked, it is hidden, which also keeps its controls out of reach.
  const park = () => {
    node.dataset.parked = "";
  };
  if (target === null) {
    park();
    return;
  }
  let frame = 0;
  const place = () => {
    frame = 0;
    const box = target.getBoundingClientRect();
    if (box.width < 1 || box.height < 1) {
      park();
      return;
    }
    delete node.dataset.parked;
    Object.assign(node.style, {
      left: `${box.left}px`,
      top: `${box.top}px`,
      width: `${box.width}px`,
      height: `${box.height}px`,
    });
  };
  const schedule = () => {
    if (frame === 0) frame = requestAnimationFrame(place);
  };
  place();
  const observer = new ResizeObserver(schedule);
  observer.observe(target);
  observer.observe(document.body);
  window.addEventListener("resize", schedule);
  document.addEventListener("scroll", schedule, true);
  document.addEventListener("transitionend", schedule, true);
  document.addEventListener("animationend", schedule, true);
  return () => {
    cancelAnimationFrame(frame);
    observer.disconnect();
    window.removeEventListener("resize", schedule);
    document.removeEventListener("scroll", schedule, true);
    document.removeEventListener("transitionend", schedule, true);
    document.removeEventListener("animationend", schedule, true);
  };
}

/** The panel a session's frame is drawn in, for asking full screen of it. */
export function activityPanelOf(slot: SessionSlot): HTMLElement | null {
  return document.querySelector<HTMLElement>(`.act-panel[data-slot="${slot}"]`);
}

/**
 * The running activities' frames, drawn once for the whole app. Each is laid
 * over the slot its view publishes (the call's stage, or the room on the
 * messages side) and hidden, still running, while no slot shows.
 */
export function ActivityHost() {
  const { voice, personal, slots } = useActivities();
  return (
    <>
      {voice === null ? null : <ActivityPanel key={voice.access} slot="voice" session={voice} target={slots.voice} />}
      {personal === null ? null : (
        <ActivityPanel key={personal.access} slot="personal" session={personal} target={slots.personal} />
      )}
    </>
  );
}

interface PanelProps {
  slot: SessionSlot;
  session: ActivitySession;
  target: HTMLElement | null;
}

/**
 * One activity, with no bar of its own above it. In a call its controls go
 * into the call's bar: its name and its own controls in the bar's free column
 * on the left, the way out at the right end. On your own they stand under the
 * activity. In full screen a small dock comes up when the pointer reaches the
 * bottom edge and drops again when it leaves; the activity may hold that edge
 * (`shell.dock.hold()`), and then a handle, the keyboard and Esc still reach it.
 */
export function ActivityPanel({ slot, session, target }: PanelProps) {
  const { t } = useLanguage();
  const activities = useActivities();
  const { leave, lose, openPicker, end, bars } = activities;
  const ui = useUi();
  const call = useVoiceCall();
  const { cloneLocalCamera } = useVoiceLive();
  const { user } = useSession();
  const { members, channelName } = useDirectory();
  const ports = usePorts();
  const confirm = useConfirm();
  const voiceSettings = useSyncExternalStore(
    ports.voiceSettings.subscribe,
    ports.voiceSettings.getSnapshot,
    ports.voiceSettings.getSnapshot,
  );
  const [panel, setPanel] = useState<HTMLElement | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [consent, setConsent] = useState<Consent | null>(null);
  const asked = useRef(0);
  const [fullscreen, setFullscreen] = useState(false);
  const [gameVolume, setGameVolume] = useState(100);
  const [gameMuted, setGameMuted] = useState(false);
  const [reload, setReload] = useState(0);
  useLayoutEffect(() => cover(panel, target), [panel, target]);

  // ---------- The dock ----------
  const always = useDockAlways();
  const [holding, setHolding] = useState(false);
  const [dockShown, setDockShown] = useState(false);
  const [dockNode, setDockNode] = useState<HTMLElement | null>(null);
  const dockElement = useRef<HTMLElement | null>(null);
  const attachDock = useCallback((node: HTMLElement | null) => {
    dockElement.current = node;
    setDockNode(node);
  }, []);
  const touch = useSyncExternalStore(subscribeHover, touchOnly, () => false);
  const [dockHeight, setDockHeight] = useState(0);
  const held = fullscreen && holding && !always;
  const dockTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const lastPeek = useRef(Number.NEGATIVE_INFINITY);
  const fullscreenNow = useRef(false);
  const holdNow = useRef(false);
  const menusOpen = useRef(false);
  const overDock = useRef(false);
  useEffect(() => {
    holdNow.current = holding && !always;
  }, [holding, always]);
  useEffect(() => {
    menusOpen.current = menuOpen || settingsOpen;
  }, [menuOpen, settingsOpen]);
  useEffect(() => () => clearTimeout(dockTimer.current), []);

  const stopDockTimer = useCallback(() => {
    clearTimeout(dockTimer.current);
    dockTimer.current = undefined;
  }, []);
  // The pointer on it, an open menu or a focused control keeps the dock up.
  const dropDock = useCallback(() => {
    stopDockTimer();
    if (overDock.current || menusOpen.current || dockElement.current?.contains(document.activeElement)) return;
    setDockShown(false);
  }, [stopDockTimer]);
  const showDock = useCallback(
    (forMs?: number) => {
      stopDockTimer();
      setDockShown(true);
      if (forMs !== undefined) dockTimer.current = setTimeout(dropDock, forMs);
    },
    [stopDockTimer, dropDock],
  );
  const dropDockSoon = useCallback(() => {
    stopDockTimer();
    dockTimer.current = setTimeout(dropDock, DOCK_LINGER_MS);
  }, [stopDockTimer, dropDock]);

  // Full screen is the browser's to end (Esc, the system's own controls), so
  // the panel follows its event rather than its own last click. Entering it
  // shows the dock for a moment, unless the activity holds the edge.
  useEffect(() => {
    const follow = () => {
      const entered = panel !== null && document.fullscreenElement === panel;
      fullscreenNow.current = entered;
      overDock.current = false;
      setFullscreen(entered);
      setMenuOpen(false);
      setSettingsOpen(false);
      if (entered && !holdNow.current) showDock(DOCK_PEEK_MS);
      else {
        stopDockTimer();
        setDockShown(false);
      }
    };
    document.addEventListener("fullscreenchange", follow);
    return () => document.removeEventListener("fullscreenchange", follow);
  }, [panel, showDock, stopDockTimer]);

  // A click in the activity never reaches this page; the focus leaving it
  // does. It closes the menus, and in full screen lets the dock drop.
  useEffect(() => {
    const away = () => {
      setMenuOpen(false);
      setSettingsOpen(false);
      if (fullscreenNow.current) dropDockSoon();
    };
    window.addEventListener("blur", away);
    return () => window.removeEventListener("blur", away);
  }, [dropDockSoon]);

  useEffect(() => {
    if (dockNode === null || panel === null) return;
    const measure = () => setDockHeight(Math.max(0, Math.round(panel.clientHeight - dockNode.offsetTop)));
    const observer = new ResizeObserver(measure);
    observer.observe(dockNode);
    observer.observe(panel);
    return () => observer.disconnect();
  }, [dockNode, panel]);

  const onDock = useCallback(
    (request: DockRequest) => {
      // A hold takes the dock down at once, unless the player is using it.
      if (request === "hold") {
        setHolding(true);
        if (fullscreenNow.current && !dockAlways()) dropDock();
      } else if (request !== "peek") setHolding(false);
      else {
        const now = Date.now();
        if (!fullscreenNow.current || now - lastPeek.current < DOCK_PEEK_GAP_MS) return;
        lastPeek.current = now;
        showDock(DOCK_PEEK_MS);
      }
    },
    [showDock, dropDock],
  );
  const visible = fullscreen && (dockShown || (touch && !held));
  const dock = useMemo<DockState>(
    () => ({ fullscreen, visible, held, height: visible ? dockHeight : 0 }),
    [fullscreen, visible, held, dockHeight],
  );
  const shell = useMemo<FrameShell>(() => ({ dock, onDock }), [dock, onDock]);

  // Closing a menu while the pointer is elsewhere lets the dock drop.
  const changeMenu = (open: boolean) => {
    setMenuOpen(open);
    if (!open && fullscreen) dropDockSoon();
  };
  const changeSettings = (open: boolean) => {
    setSettingsOpen(open);
    if (!open && fullscreen) dropDockSoon();
  };

  // ---------- The frame ----------
  // A device the activity asks for is asked about with the app's own card,
  // drawn inside the panel while it has the screen, so full screen shows it.
  const ask = useCallback(
    (kind: DeviceKind, signal: AbortSignal) =>
      new Promise<boolean>((resolve) => {
        asked.current += 1;
        const id = asked.current;
        const answer = (allowed: boolean) => {
          setConsent((current) => (current?.id === id ? null : current));
          resolve(allowed);
        };
        signal.addEventListener("abort", () => answer(false), { once: true });
        setConsent({ id, kind, answer });
      }),
    [],
  );

  const voice = slot === "voice";
  const selfId = user?.id ?? "";
  const summary = voice ? (activities.live.get(session.place) ?? null) : null;
  const hosting = summary !== null && summary.host === selfId;

  const { place, install, access } = session;
  const realtime = useOptionalRealtime();
  const room = useCallback(
    (action: Record<string, unknown>, signal: AbortSignal) =>
      activitiesApi.act(place, "room", { install, access, action }, signal),
    [place, install, access],
  );
  const liveRoom = useCallback(
    async (action: Record<string, unknown>, signal: AbortSignal) => {
      if (!realtime?.activityRoom) return room(action, signal);
      if (realtime.hasSessionEnded?.()) throw Object.assign(new Error("Session ended"), { status: 401 });
      try {
        return await realtime.activityRoom(place, { install, access, action });
      } catch (error) {
        // A legal state patch can exceed the hub's frame limit. This error is
        // raised locally before sending, so falling back cannot repeat a write.
        if (error instanceof Error && "code" in error && error.code === "frame_too_large") {
          return room(action, signal);
        }
        throw error;
      }
    },
    [realtime, room, place, install, access],
  );
  const camera = useCallback(
    () => new ActivityCameraTransport(realtime, selfId, place, { install, access }),
    [realtime, selfId, place, install, access],
  );
  const lost = useCallback(() => lose(slot, access), [lose, slot, access]);

  const leaveFullscreen = async () => {
    if (document.fullscreenElement) await document.exitFullscreen?.().catch(() => undefined);
  };

  // Ending it for everyone asks first, with the app's card, which full
  // screen would hide.
  const endForAll = async () => {
    setMenuOpen(false);
    await leaveFullscreen();
    await confirm({
      title: t("activities.endTitle", { name: session.title }),
      description: t("activities.endDetail", {
        count: summary?.participants.length ?? 1,
        channel: channelName(session.place),
      }),
      confirmLabel: t("activities.end"),
      cancelLabel: t("common.cancel"),
      busyLabel: t("activities.ending"),
      danger: true,
      run: () => end(session.place),
      failure: t("activities.error.generic"),
    });
  };

  const toggleFullscreen = () => {
    if (document.fullscreenElement) {
      void document.exitFullscreen?.().catch(() => undefined);
      return;
    }
    void panel?.requestFullscreen?.().catch(() => undefined);
  };

  const nameOf = (id: string) =>
    id === selfId ? (user?.displayName ?? "") : (members.find((member) => member.id === id)?.displayName ?? "");
  const successor = summary?.participants.find((id) => id !== selfId) ?? null;

  // A host or successor not among the server's people is left unnamed
  // rather than named as a blank.
  const hostName = summary === null || hosting ? "" : nameOf(summary.host);
  const successorName = successor === null ? "" : nameOf(successor);
  let line: string;
  if (!voice) line = t("activities.onlyYou");
  else if (summary === null) line = "";
  else if (hosting) line = t("activities.insideHostYou", { count: summary.participants.length });
  else if (hostName !== "") line = t("activities.insideHost", { count: summary.participants.length, name: hostName });
  else line = t("activities.inside", { count: summary.participants.length });

  const fullscreenTitle = fullscreen ? t("voiceBar.fullscreenExit") : t("activities.fullscreen");
  const leaveLabel = voice ? t("activities.leave") : t("activities.close");
  const callControls = voice && call.call !== null;
  const requested = DEVICES.filter((kind) => session.permissions.includes(`media.${kind}`));
  const level = gameMuted ? 0 : gameVolume / 100;

  const bar = ({ devices, stopDevice, attribution }: FrameParts): ReactNode => {
    const stops = devices.map((kind) => (
      <button
        key={kind}
        type="button"
        className="vc-btn on act-rec"
        aria-label={t(DEVICE[kind].stop)}
        title={t(DEVICE[kind].stop)}
        onClick={() => stopDevice(kind)}
      >
        <Icon name={DEVICE[kind].icon} size={19} />
      </button>
    ));
    const settings = (
      <span className="act-set">
        <Popover
          open={settingsOpen}
          onOpenChange={changeSettings}
          className="vc-menu act-settings"
          renderTrigger={(trigger) => (
            <button
              type="button"
              className={settingsOpen ? "vc-btn on" : "vc-btn"}
              aria-label={t("activities.settings")}
              title={t("activities.settings")}
              {...trigger}
            >
              <Icon name="sliders" size={19} />
            </button>
          )}
        >
          <ActivitySettings
            voice={voice}
            volume={gameVolume}
            muted={gameMuted}
            onVolume={(value) => {
              setGameVolume(value);
              setGameMuted(false);
            }}
            onMuted={setGameMuted}
            callVolume={voiceSettings.outputVolume}
            onCallVolume={(outputVolume) => ports.voiceSettings.update({ outputVolume })}
            requested={requested}
            devices={devices}
            onStop={stopDevice}
            always={always}
            onAlways={setDockAlways}
            onReload={() => {
              setSettingsOpen(false);
              setReload((count) => count + 1);
            }}
            onVoiceSettings={async () => {
              setSettingsOpen(false);
              await leaveFullscreen();
              ui.openModal("voice");
            }}
          />
        </Popover>
      </span>
    );

    if (fullscreen) {
      return (
        <>
          {held ? null : (
            // The frame keeps the pointer to itself, so the edge under it is
            // what brings the dock up. A held edge is the activity's.
            <div className="act-wake" aria-hidden="true" onPointerEnter={() => showDock()} onPointerMove={() => showDock()} onPointerLeave={dropDockSoon} />
          )}
          {held && !visible ? (
            <button
              type="button"
              className="act-grip"
              aria-label={t("activities.showDock")}
              title={t("activities.showDock")}
              onClick={() => showDock()}
            >
              <Icon name="caretUp" size={14} />
            </button>
          ) : null}
          <div
            ref={attachDock}
            className="act-dock"
            role="group"
            aria-label={t("activities.dock")}
            onPointerEnter={() => {
              overDock.current = true;
              stopDockTimer();
            }}
            onPointerLeave={() => {
              overDock.current = false;
              dropDockSoon();
            }}
            onFocus={() => showDock()}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) dropDockSoon();
            }}
          >
            {callControls ? (
              <>
                <button
                  type="button"
                  className={call.call?.muted ? "vc-btn muted" : "vc-btn"}
                  aria-pressed={call.call?.muted ?? false}
                  aria-label={t("voice.microphone")}
                  title={call.call?.muted ? t("voiceBar.unmute") : t("voiceBar.mute")}
                  onClick={call.toggleMute}
                >
                  <Icon name={call.call?.muted ? "micSlash" : "mic"} size={18} />
                </button>
                <button
                  type="button"
                  className={call.call?.deafened ? "vc-btn muted" : "vc-btn"}
                  aria-pressed={call.call?.deafened ?? false}
                  aria-label={t("shell.muteSound")}
                  title={call.call?.deafened ? t("shell.unmuteSound") : t("shell.muteSound")}
                  onClick={call.toggleDeafen}
                >
                  <Icon name={call.call?.deafened ? "headphonesSlash" : "headphones"} size={18} />
                </button>
                <span className="act-dock-sep" aria-hidden="true" />
              </>
            ) : null}
            {stops}
            {settings}
            {attribution === null ? null : <span className="act-dock-mark">{attribution}</span>}
            <span className="act-dock-sep" aria-hidden="true" />
            <button type="button" className="vc-btn" aria-label={fullscreenTitle} title={fullscreenTitle} onClick={toggleFullscreen}>
              <Icon name="collapse" size={18} />
            </button>
            <button
              type="button"
              className="vc-btn act-dock-leave"
              aria-label={leaveLabel}
              title={leaveLabel}
              onClick={() => leave(slot)}
            >
              <Icon name="signout" size={18} />
            </button>
          </div>
        </>
      );
    }

    const identity = (
      <span className="act-id" title={line === "" ? session.title : `${session.title} · ${line}`}>
        <ActivityIcon install={session.install} name={session.title} icon={session.icon} size={34} />
        <span className="act-id-text">
          <b>{session.title}</b>
          {line === "" ? null : <span>{line}</span>}
        </span>
      </span>
    );
    const tools = (
      <span className="vc-grp act-tools">
        <button
          type="button"
          className={gameMuted ? "vc-btn muted" : "vc-btn"}
          aria-pressed={gameMuted}
          aria-label={t("activities.gameAudio")}
          title={gameMuted ? t("activities.gameUnmute") : t("activities.gameMute")}
          onClick={() => setGameMuted(!gameMuted)}
        >
          <Icon name={gameMuted ? "volumeCross" : "sound"} size={19} />
        </button>
        {stops}
        {settings}
      </span>
    );
    const exit = (
      <span className="act-leave">
        <button type="button" className="act-leave-btn" aria-label={leaveLabel} title={leaveLabel} onClick={() => leave(slot)}>
          <Icon name="signout" size={16} />
          <span>{voice ? t("activities.leaveShort") : t("activities.close")}</span>
        </button>
        {hosting ? (
          <span className="act-more">
            <Popover
              open={menuOpen}
              onOpenChange={changeMenu}
              className="act-menu"
              renderTrigger={(trigger) => (
                <button
                  type="button"
                  className="act-more-btn"
                  aria-label={t("activities.more")}
                  title={t("activities.more")}
                  {...trigger}
                >
                  <Icon name="caretUp" size={12} />
                </button>
              )}
            >
              <button
                type="button"
                className="qm-more"
                onClick={() => {
                  setMenuOpen(false);
                  openPicker({ scope: "voice", channelId: session.place, change: true });
                }}
              >
                <span className="qm-more-icon" aria-hidden="true">
                  <Icon name="game" size={15} />
                </span>
                {t("activities.change")}
              </button>
              <button type="button" className="qm-more danger" onClick={() => void endForAll()}>
                <span className="qm-more-icon" aria-hidden="true">
                  <Icon name="stop" size={15} />
                </span>
                {t("activities.endForAll")}
              </button>
              <p className="qm-note">
                {successor === null
                  ? t("activities.handoverNobody", { activity: session.title })
                  : successorName === ""
                    ? t("activities.handoverNext", { activity: session.title })
                    : t("activities.handover", { name: successorName, activity: session.title })}
              </p>
            </Popover>
          </span>
        ) : null}
      </span>
    );

    // In a call the controls go into the call's bar; without one in view they
    // stand under the activity, as they do on your own.
    if (voice && (bars.start !== null || bars.end !== null)) {
      return (
        <>
          {bars.start === null
            ? null
            : createPortal(
                <>
                  {identity}
                  {tools}
                </>,
                bars.start,
              )}
          {bars.end === null
            ? null
            : createPortal(
                <>
                  {attribution}
                  <span className="act-sep" aria-hidden="true" />
                  {exit}
                </>,
                bars.end,
              )}
        </>
      );
    }
    return (
      <footer className="act-foot">
        {voice ? null : (
          <button
            type="button"
            className="burger"
          aria-label={t(ui.space === "dm" ? "common.showConversations" : "room.showChannels")}
          aria-expanded={ui.railOpen}
            onClick={ui.toggleRail}
          >
            <Icon name="list" size={18} />
          </button>
        )}
        {identity}
        {tools}
        <span className="act-foot-end">
          {attribution}
          <button type="button" className="vc-plain" aria-label={fullscreenTitle} title={fullscreenTitle} onClick={toggleFullscreen}>
            <Icon name="expand" size={18} />
          </button>
          <span className="act-sep" aria-hidden="true" />
          {exit}
        </span>
      </footer>
    );
  };

  return (
    <section
      ref={setPanel}
      className="act-panel"
      data-slot={slot}
      data-dock={visible ? "shown" : undefined}
      data-held={held ? "" : undefined}
      aria-label={session.title}
    >
      <ActivityFrame
        src={session.src}
        title={session.title}
        permissions={session.permissions}
        embedHosts={session.embedHosts}
        muted={call.call?.muted ?? false}
        cameraOn={call.call?.cameraOn}
        room={room}
        liveRoom={realtime?.activityRoom ? liveRoom : undefined}
        realtime={realtime}
        camera={camera}
        cloneLocalCamera={cloneLocalCamera}
        onLost={lost}
        bar={bar}
        ask={ask}
        volume={level}
        reload={reload}
        shell={shell}
      />
      {consent === null
        ? null
        : createPortal(
            <ConfirmDialog
              open
              questionKey={consent.id}
              title={t(CONSENT[consent.kind].title, { name: session.title })}
              description={t(CONSENT[consent.kind].description)}
              subject={{
                kind: "item",
                name: session.title,
                detail: t(CONSENT[consent.kind].subject),
                mark: session.icon === null ? { icon: "game" } : { image: session.icon },
              }}
              confirmLabel={t(CONSENT[consent.kind].allow)}
              cancelLabel={t(CONSENT[consent.kind].deny)}
              onConfirm={() => consent.answer(true)}
              onCancel={() => consent.answer(false)}
            />,
            fullscreen && panel !== null ? panel : document.body,
          )}
    </section>
  );
}

interface SettingsProps {
  voice: boolean;
  /** 0–100. */
  volume: number;
  muted: boolean;
  onVolume: (value: number) => void;
  onMuted: (muted: boolean) => void;
  /** The call's output volume, 0–100: the same setting as in voice settings. */
  callVolume: number;
  onCallVolume: (value: number) => void;
  /** The devices the activity may ask for. */
  requested: readonly DeviceKind[];
  /** The ones it has open now. */
  devices: readonly DeviceKind[];
  onStop: (kind: DeviceKind) => void;
  always: boolean;
  onAlways: (always: boolean) => void;
  onReload: () => void;
  onVoiceSettings: () => void;
}

/** Everything about this activity that is only yours: its sound, the call's, the devices, the dock. */
function ActivitySettings({
  voice,
  volume,
  muted,
  onVolume,
  onMuted,
  callVolume,
  onCallVolume,
  requested,
  devices,
  onStop,
  always,
  onAlways,
  onReload,
  onVoiceSettings,
}: SettingsProps) {
  const { t } = useLanguage();
  return (
    <div className="act-settings-body">
      <div className="qm-head">
        <b>{t("activities.settings")}</b>
        <span>{t("activities.settingsDetail")}</span>
      </div>
      <label className="act-field">
        <span>
          {t("activities.gameAudio")}
          <output>{muted ? t("activities.gameMuted") : `${volume}%`}</output>
        </span>
        <input
          type="range"
          aria-label={t("activities.gameAudio")}
          min={0}
          max={100}
          step={1}
          value={volume}
          onChange={(event) => onVolume(Number(event.target.value))}
        />
      </label>
      <label className="act-check">
        <input type="checkbox" checked={muted} onChange={(event) => onMuted(event.target.checked)} />
        <span>{t("activities.gameMute")}</span>
      </label>
      {voice ? (
        <label className="act-field act-rule">
          <span>
            {t("activities.callVolume")}
            <output>{callVolume}%</output>
          </span>
          <input
            type="range"
            aria-label={t("activities.callVolume")}
            min={0}
            max={100}
            step={1}
            value={callVolume}
            onChange={(event) => onCallVolume(Number(event.target.value))}
          />
        </label>
      ) : null}
      {requested.length === 0 ? null : (
        <div className="act-rule">
          <div className="qm-caption">{t("activities.devicesUsed")}</div>
          {requested.map((kind) => (
            <div key={kind} className="act-device">
              <Icon name={DEVICE[kind].icon} size={16} />
              <span>{t(DEVICE[kind].name)}</span>
              {devices.includes(kind) ? (
                <button type="button" onClick={() => onStop(kind)}>
                  {t(DEVICE[kind].stop)}
                </button>
              ) : (
                <small>{t("activities.deviceIdle")}</small>
              )}
            </div>
          ))}
        </div>
      )}
      <label className="act-check act-rule">
        <input type="checkbox" checked={always} onChange={(event) => onAlways(event.target.checked)} />
        <span>
          {t("activities.dockAlways")}
          <small>{t("activities.dockAlwaysDetail")}</small>
        </span>
      </label>
      <div className="act-rule">
        <button type="button" className="qm-more" onClick={onReload}>
          <span className="qm-more-icon" aria-hidden="true">
            <Icon name="refresh" size={15} />
          </span>
          {t("activities.reload")}
        </button>
        {voice ? (
          <button type="button" className="qm-more" onClick={onVoiceSettings}>
            <span className="qm-more-icon" aria-hidden="true">
              <Icon name="gear" size={15} />
            </span>
            {t("activities.voiceSettings")}
          </button>
        ) : null}
      </div>
    </div>
  );
}
