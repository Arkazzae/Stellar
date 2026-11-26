import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { Icon } from "@stellarr/icons";
import { ActivityIcon } from "@/components/activities/ActivityIcon";
import { Avatar } from "@/components/primitives/Avatar";
import { ConfirmDialog, useConfirm } from "@/components/primitives/ConfirmDialog";
import { Popover } from "@/components/primitives/Popover";
import { useChromeIdle } from "@/components/voice/useChromeIdle";
import { activitiesApi } from "@/lib/activities/api";
import {
  useActivities,
  useDirectory,
  useLanguage,
  useSession,
  useUi,
  useVoiceCall,
  type ActivitySession,
} from "@/state/hooks";
import type { SessionSlot } from "@/state/ActivitiesProvider";

import { ActivityFrame, type DeviceKind, type FrameParts, type FramePlayer } from "./ActivityFrame";

/** The activity's bar fades after this long without the pointer moving, in full screen. */
const BAR_REST_MS = 2000;
/** How long full screen says how to leave it. */
const HINT_MS = 3000;
/** Faces shown in the bar before the rest are left to the count. */
const FACES = 4;

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

/** One activity: its own bar, then its frame. The bar is what full screen and cinema keep. */
export function ActivityPanel({ slot, session, target }: PanelProps) {
  const { t } = useLanguage();
  const activities = useActivities();
  const { leave, lose, openPicker, end } = activities;
  const ui = useUi();
  const call = useVoiceCall();
  const { user } = useSession();
  const { members, channelName } = useDirectory();
  const confirm = useConfirm();
  const [panel, setPanel] = useState<HTMLElement | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [consent, setConsent] = useState<Consent | null>(null);
  const asked = useRef(0);
  const [fullscreen, setFullscreen] = useState(false);
  const [hint, setHint] = useState(false);
  const chrome = useChromeIdle(BAR_REST_MS);
  const { wake } = chrome;
  useLayoutEffect(() => cover(panel, target), [panel, target]);

  // Full screen is the browser's to end (Esc, the system's own controls), so
  // the panel follows its event rather than its own last click.
  useEffect(() => {
    const follow = () => {
      const entered = panel !== null && document.fullscreenElement === panel;
      setFullscreen(entered);
      setHint(entered);
      if (entered) wake();
    };
    document.addEventListener("fullscreenchange", follow);
    return () => document.removeEventListener("fullscreenchange", follow);
  }, [panel, wake]);

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
  useEffect(() => {
    if (!hint) return;
    const timer = setTimeout(() => setHint(false), HINT_MS);
    return () => clearTimeout(timer);
  }, [hint]);

  const voice = slot === "voice";
  const selfId = user?.id ?? "";
  const summary = voice ? (activities.live.get(session.place) ?? null) : null;
  const hosting = summary !== null && summary.host === selfId;

  const { place, install, access } = session;
  const room = useCallback(
    (action: Record<string, unknown>, signal: AbortSignal) =>
      activitiesApi.act(place, "room", { install, access, action }, signal),
    [place, install, access],
  );
  const tile = useCallback(
    async (action: Record<string, unknown>, signal: AbortSignal) =>
      (await activitiesApi.act(place, "tile", { install, access, action }, signal)) as Record<string, string | null>,
    [place, install, access],
  );
  const lost = useCallback(() => lose(slot, access), [lose, slot, access]);

  // Ending it for everyone asks first, with the app's card, which full
  // screen would hide.
  const endForAll = async () => {
    setMenuOpen(false);
    if (document.fullscreenElement) await document.exitFullscreen?.().catch(() => undefined);
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

  const faces = (players: readonly FramePlayer[]): FramePlayer[] => {
    if (players.length > 0) return players.slice(0, FACES);
    return (summary?.participants ?? []).slice(0, FACES).map((id) => {
      const member = id === selfId ? user : members.find((candidate) => candidate.id === id);
      return {
        id,
        label: member?.displayName ?? "",
        ...(member?.avatarUrl ? { avatarUrl: member.avatarUrl } : {}),
        connected: true,
      };
    });
  };

  const fullscreenTitle = fullscreen ? t("voiceBar.fullscreenExit") : t("activities.fullscreen");
  const callControls = voice && fullscreen && call.call !== null;

  const bar = ({ volume, devices, players }: FrameParts) => (
    <header className="act-bar" onPointerMove={wake} onFocus={wake}>
      {voice ? null : (
        <button
          type="button"
          className="burger"
          aria-label={t("common.showConversations")}
          aria-expanded={ui.railOpen}
          onClick={ui.toggleRail}
        >
          <Icon name="list" size={18} />
        </button>
      )}
      <ActivityIcon install={session.install} name={session.title} icon={session.icon} size={28} />
      <span className="act-bar-title">
        <b>{session.title}</b>
        {line === "" ? null : <span>{line}</span>}
      </span>
      {voice ? (
        <span className="act-faces" aria-hidden="true">
          {faces(players).map((person) => (
            <span key={person.id} className="act-face" title={person.label}>
              <Avatar
                id={person.id}
                displayName={person.label}
                avatarUrl={person.avatarUrl}
                size="xs"
                offline={!person.connected}
              />
            </span>
          ))}
        </span>
      ) : null}
      <span className="act-bar-tools">
        {callControls ? (
          <span className="act-bar-call">
            <button
              type="button"
              className={call.call?.muted ? "vc-plain danger" : "vc-plain"}
              aria-pressed={call.call?.muted ?? false}
              aria-label={t("voice.microphone")}
              title={call.call?.muted ? t("voiceBar.unmute") : t("voiceBar.mute")}
              onClick={call.toggleMute}
            >
              <Icon name={call.call?.muted ? "micSlash" : "mic"} size={18} />
            </button>
            <button
              type="button"
              className={call.call?.deafened ? "vc-plain danger" : "vc-plain"}
              aria-pressed={call.call?.deafened ?? false}
              aria-label={t("shell.muteSound")}
              title={call.call?.deafened ? t("shell.unmuteSound") : t("shell.muteSound")}
              onClick={call.toggleDeafen}
            >
              <Icon name={call.call?.deafened ? "headphonesSlash" : "headphones"} size={18} />
            </button>
          </span>
        ) : null}
        {volume}
        {devices}
        {voice && !fullscreen ? (
          <button
            type="button"
            className="vc-plain"
            aria-pressed={ui.cinema}
            aria-label={ui.cinema ? t("voiceBar.cinemaExit") : t("voiceBar.cinema")}
            title={ui.cinema ? t("voiceBar.cinemaExit") : t("voiceBar.cinema")}
            onClick={ui.toggleCinema}
          >
            <Icon name={ui.cinema ? "cinemaOff" : "cinema"} size={ui.cinema ? 20 : 18} />
          </button>
        ) : null}
        <button
          type="button"
          className="vc-plain"
          aria-pressed={fullscreen}
          aria-label={fullscreenTitle}
          title={fullscreenTitle}
          onClick={toggleFullscreen}
        >
          <Icon name={fullscreen ? "collapse" : "expand"} size={17} />
        </button>
        <span className="act-leave">
          <button type="button" className="act-leave-btn" onClick={() => leave(slot)}>
            {voice ? t("activities.leave") : t("activities.close")}
          </button>
          {hosting ? (
            <span className="act-more">
              <Popover
                open={menuOpen}
                onOpenChange={setMenuOpen}
                className="act-menu"
                renderTrigger={(trigger) => (
                  <button
                    type="button"
                    className="act-more-btn"
                    aria-label={t("activities.more")}
                    title={t("activities.more")}
                    {...trigger}
                  >
                    <Icon name="caret" size={12} />
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
                <button
                  type="button"
                  className="qm-more danger"
                  onClick={() => void endForAll()}
                >
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
        {callControls ? (
          <button
            type="button"
            className="act-hang"
            aria-label={t("voiceBar.disconnect")}
            title={t("voiceBar.disconnect")}
            onClick={ui.leaveCall}
          >
            <Icon name="phoneX" size={18} />
          </button>
        ) : null}
      </span>
    </header>
  );

  return (
    <section
      ref={setPanel}
      className="act-panel"
      data-slot={slot}
      data-chrome={fullscreen && chrome.idle ? "idle" : "awake"}
      aria-label={session.title}
      onKeyDown={wake}
    >
      <ActivityFrame
        src={session.src}
        title={session.title}
        permissions={session.permissions}
        youtube={session.youtube}
        muted={call.call?.muted ?? false}
        cameraOn={call.call?.cameraOn}
        room={room}
        tile={tile}
        onLost={lost}
        bar={bar}
        ask={ask}
      />
      {/* The frame keeps the pointer to itself, so in full screen the edge
          above it is what brings the bar back. */}
      {fullscreen ? <div className="act-wake" aria-hidden="true" onPointerMove={wake} onPointerDown={wake} /> : null}
      {hint ? (
        <p className="act-hint" role="status">
          {t("activities.escHint")}
        </p>
      ) : null}
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
