import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import {
  activitiesApi,
  ActivityRequestError,
  deviceClass,
  PERSONAL,
  type ActivitySummary,
  type Lease,
} from "@/lib/activities/api";
import { refusalText } from "@/lib/activities/reasons";

import { useDirectoryContext } from "./DirectoryProvider";
import { useLanguage } from "./LanguageProvider";
import { useOptionalRealtime } from "./PortsProvider";
import { useSessionContext } from "./SessionProvider";
import { useUiContext } from "./UiProvider";
import { useVoiceContext } from "./VoiceProvider";

/** Which of the two seats a session holds: the call's activity, or your own. */
export type SessionSlot = "voice" | "personal";
/** The two places in the call's bar the voice activity's controls go: its own column, and by the view's controls. */
export type BarSlot = "start" | "end";

/** A seat this client holds in a running activity, and the frame it opens. */
export interface ActivitySession {
  /** Where it runs: a voice channel's id, or `PERSONAL`. */
  place: string;
  install: string;
  access: string;
  src: string;
  title: string;
  icon: string | null;
  permissions: string[];
  /** External embed domains approved in the activity manifest. */
  embedHosts: string[];
}

/** What the picker is asked to offer, and for whom. */
export type PickerRequest =
  | { scope: "voice"; channelId: string; change: boolean }
  | { scope: "personal" };

/** The installed activity the picker started, as far as the stage needs it. */
export interface PickedActivity {
  install: string;
  name: string;
  icon: string | null;
  seats: number | null;
}

export interface ActivitiesValue {
  /** What runs on each voice channel this client has heard of, by channel. */
  live: ReadonlyMap<string, ActivitySummary>;
  /** The activity of the call you are in, when you are inside it. */
  voice: ActivitySession | null;
  /** The activity you run for yourself, on screen or in the background. */
  personal: ActivitySession | null;
  /** The channel whose activity a one-click join is getting you into. */
  joining: string | null;
  /** Why the last one-click join failed, for the channel it was about. */
  joinFailure: { channelId: string; text: string } | null;
  /** An activity that ended under you, said once for a few seconds. */
  ended: { place: string; name: string } | null;
  picker: PickerRequest | null;
  openPicker: (request: PickerRequest) => void;
  closePicker: () => void;
  /** Gets you into what runs on the channel, joining the channel first if need be. */
  join: (channelId: string) => void;
  /** Starts an activity, or with `replace` swaps the channel's one for it. Rejects with the reason. */
  launch: (place: string, picked: PickedActivity, options?: { replace?: boolean }) => Promise<void>;
  leave: (slot: SessionSlot) => void;
  /** Ends the channel's activity for everyone; the host's call. */
  end: (channelId: string) => Promise<void>;
  /** The frame learnt that its seat is gone. */
  lose: (slot: SessionSlot, access: string) => void;
  /** Where each session's frame is shown; null while nothing on screen holds a place for it. */
  slots: Readonly<Record<SessionSlot, HTMLElement | null>>;
  /** The ref a view puts on the element the frame should cover. */
  publishSlot: Readonly<Record<SessionSlot, (element: HTMLElement | null) => void>>;
  /** Where the call's bar holds the voice activity's controls; null while it holds none. */
  bars: Readonly<Record<BarSlot, HTMLElement | null>>;
  /** The refs the call's bar puts on those places. */
  publishBar: Readonly<Record<BarSlot, (element: HTMLElement | null) => void>>;
}

/** A seat lapses after a minute without word; this renews it well before. */
export const LEASE_RENEW_MS = 10_000;
/** How long a failure or an ending stays said. */
const NOTICE_MS = 6000;
/** A seat in the call can lag behind the click that joined it. */
const SEAT_RETRIES = 4;
const SEAT_RETRY_MS = 500;
/** How long a join waits for the call it started to connect. */
const JOIN_WAIT_MS = 20_000;

const NO_LIVE: ReadonlyMap<string, ActivitySummary> = new Map();
const NO_SLOTS: Readonly<Record<SessionSlot, HTMLElement | null>> = { voice: null, personal: null };
const NO_BARS: Readonly<Record<BarSlot, HTMLElement | null>> = { start: null, end: null };

function sameSummary(a: ActivitySummary | undefined, b: ActivitySummary | null): boolean {
  if (a === undefined || b === null) return a === undefined && b === null;
  return (
    a.install === b.install &&
    a.name === b.name &&
    a.icon === b.icon &&
    a.host === b.host &&
    a.seats === b.seats &&
    a.participants.length === b.participants.length &&
    a.participants.every((person, index) => person === b.participants[index])
  );
}

/** The map with one channel's activity set or cleared; the same map when nothing changed. */
function withLive(
  current: ReadonlyMap<string, ActivitySummary>,
  channelId: string,
  activity: ActivitySummary | null,
): ReadonlyMap<string, ActivitySummary> {
  if (sameSummary(current.get(channelId), activity)) return current;
  const next = new Map(current);
  if (activity === null) next.delete(channelId);
  else next.set(channelId, activity);
  return next;
}

const leaseOf = (session: ActivitySession): Lease => ({ install: session.install, access: session.access });

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

const ActivitiesContext = createContext<ActivitiesValue | null>(null);

/**
 * The activities this client runs, and what runs where.
 *
 * It sits above the rooms, so a session outlives the view that started it:
 * going to a conversation or another channel hides the frame and keeps the
 * seat, renewed every ten seconds, instead of dropping it unannounced. The
 * frames themselves are drawn by `ActivityHost`, once, over whichever slot a
 * view publishes.
 */
export function ActivitiesProvider({ children }: { children: ReactNode }) {
  const { t } = useLanguage();
  const realtime = useOptionalRealtime();
  const { server } = useDirectoryContext();
  const { connection, user } = useSessionContext();
  const voice = useVoiceContext();
  const ui = useUiContext();

  const serverId = server.id;
  const selfId = user?.id ?? "";
  const online = connection === "connected";
  const callChannel = voice.channelId;
  const callConnected = voice.state.connection === "connected";
  const callFailed = voice.state.connection === "failed";
  const callError = voice.error;

  const [live, setLive] = useState<ReadonlyMap<string, ActivitySummary>>(NO_LIVE);
  const [voiceSession, setVoiceSession] = useState<ActivitySession | null>(null);
  const [personal, setPersonal] = useState<ActivitySession | null>(null);
  const [joining, setJoining] = useState<string | null>(null);
  const [pendingJoin, setPendingJoin] = useState<{
    channelId: string;
    from: string | null;
    at: number;
    /** What the call said was wrong before this join, so only a new word counts. */
    errorBefore: string | null;
  } | null>(null);
  const [joinFailure, setJoinFailure] = useState<{ channelId: string; text: string } | null>(null);
  const [ended, setEnded] = useState<{ place: string; name: string } | null>(null);
  const [picker, setPicker] = useState<PickerRequest | null>(null);
  const [slots, setSlots] = useState(NO_SLOTS);
  const [bars, setBars] = useState(NO_BARS);
  /** Bumped to renew the call's seat now rather than on the next beat. */
  const [beat, setBeat] = useState(0);

  // The latest of everything the callbacks below read, so they stay the same
  // functions for the provider's life and the views that take them do not
  // render for a speaking level.
  const latest = useRef({ live, voiceSession, personal, joining, selfId, callChannel, callError, ui, t });
  useLayoutEffect(() => {
    latest.current = { live, voiceSession, personal, joining, selfId, callChannel, callError, ui, t };
  });
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  /*
   * What runs on the open server's channels: asked for when the server or the
   * connection changes, and kept current by the hub's events in between. An
   * event that lands while the answer is on its way is applied over it.
   */
  const heardWhileAsking = useRef<Map<string, ActivitySummary | null> | null>(null);
  useEffect(() => {
    if (!online || serverId === "") return;
    const controller = new AbortController();
    const heard = new Map<string, ActivitySummary | null>();
    heardWhileAsking.current = heard;
    activitiesApi.live(serverId, controller.signal).then(
      (running) => {
        if (controller.signal.aborted) return;
        heardWhileAsking.current = null;
        let next: ReadonlyMap<string, ActivitySummary> = running;
        for (const [channelId, activity] of heard) next = withLive(next, channelId, activity);
        setLive(next);
      },
      () => {
        if (heardWhileAsking.current === heard) heardWhileAsking.current = null;
      },
    );
    return () => {
      controller.abort();
      if (heardWhileAsking.current === heard) heardWhileAsking.current = null;
    };
  }, [online, serverId]);

  useEffect(() => {
    if (realtime === null) return;
    return realtime.on("VoiceActivityChanged", ({ channelId, activity }) => {
      heardWhileAsking.current?.set(channelId, activity);
      setLive((current) => withLive(current, channelId, activity));
      // A swap says "nothing" before it says what replaced it, so the event
      // never ends the seat by itself: the seat is asked about at once, and
      // only the server's answer to that ends it.
      const mine = latest.current.voiceSession;
      if (mine !== null && mine.place === channelId && activity?.install !== mine.install) {
        setBeat((count) => count + 1);
      }
    });
  }, [realtime]);

  const drop = useCallback((slot: SessionSlot, access: string, announce: boolean) => {
    const set = slot === "voice" ? setVoiceSession : setPersonal;
    const current = slot === "voice" ? latest.current.voiceSession : latest.current.personal;
    if (current?.access !== access) return;
    set(null);
    if (slot === "personal") latest.current.ui.closeActivity();
    if (announce) setEnded({ place: current.place, name: current.title });
  }, []);

  // The seat is renewed for as long as the session lasts, whether its frame
  // is on screen or not; a seat the server no longer knows ends the session.
  useLease(voiceSession, beat, setLive, drop, "voice");
  useLease(personal, 0, setLive, drop, "personal");

  // Leaving the call leaves its activity: let go of during render, as soon
  // as the call says so, and told to the server after.
  const [abandoned, setAbandoned] = useState<ActivitySession | null>(null);
  if (voiceSession !== null && voiceSession.place !== callChannel) {
    setAbandoned(voiceSession);
    setVoiceSession(null);
  }
  useEffect(() => {
    if (abandoned !== null) void activitiesApi.leave(abandoned.place, leaseOf(abandoned)).catch(() => undefined);
  }, [abandoned]);

  // A join waiting for its call gives up when the call goes somewhere else.
  // Passing through no call on the way is not somewhere else.
  if (
    pendingJoin !== null &&
    callChannel !== null &&
    callChannel !== pendingJoin.from &&
    callChannel !== pendingJoin.channelId
  ) {
    setPendingJoin(null);
    setJoining(null);
  }
  // The call itself could not be joined (full, not allowed): the join ends
  // now, with the call's own reason, where any other failed join is said.
  else if (
    pendingJoin !== null &&
    ((callFailed && callChannel === pendingJoin.channelId) ||
      (!callConnected && callError !== null && callError !== pendingJoin.errorBefore))
  ) {
    setPendingJoin(null);
    setJoining(null);
    setJoinFailure({ channelId: pendingJoin.channelId, text: callError ?? t("activities.error.seat") });
  }

  useEffect(() => {
    if (ended === null) return;
    const timer = setTimeout(() => setEnded(null), NOTICE_MS);
    return () => clearTimeout(timer);
  }, [ended]);

  useEffect(() => {
    if (joinFailure === null) return;
    const timer = setTimeout(() => setJoinFailure(null), NOTICE_MS);
    return () => clearTimeout(timer);
  }, [joinFailure]);

  const enter = useCallback(async (channelId: string) => {
    setJoining(channelId);
    setJoinFailure(null);
    for (let attempt = 0; ; attempt += 1) {
      try {
        const reply = await activitiesApi.launch(channelId, { device: deviceClass() });
        if (!alive.current) return;
        const summary = latest.current.live.get(channelId);
        setEnded(null);
        setVoiceSession({
          place: channelId,
          install: reply.install,
          access: reply.access,
          src: reply.src,
          title: reply.title || summary?.name || "",
          icon: summary?.install === reply.install ? summary.icon : null,
          permissions: reply.permissions,
          embedHosts: reply.embedHosts,
        });
        break;
      } catch (cause) {
        if (!alive.current) return;
        const seatLag = cause instanceof ActivityRequestError && cause.status === 403 && cause.code === null;
        if (seatLag && attempt < SEAT_RETRIES) {
          await wait(SEAT_RETRY_MS * 2 ** attempt);
          if (!alive.current) return;
          continue;
        }
        setJoinFailure({ channelId, text: refusalText(latest.current.t, cause, deviceClass()) });
        break;
      }
    }
    setJoining(null);
  }, []);

  const join = useCallback(
    (channelId: string) => {
      const { joining: busy, voiceSession: mine, callChannel: inCall, callError: wrong, ui: view } = latest.current;
      if (busy !== null) return;
      if (mine?.place === channelId) {
        view.returnToChannel();
        return;
      }
      if (inCall === channelId) {
        view.returnToChannel();
        void enter(channelId);
        return;
      }
      // Into the channel first; the activity follows once the call connects.
      setJoining(channelId);
      setJoinFailure(null);
      setPendingJoin({ channelId, from: inCall, at: Date.now(), errorBefore: wrong });
      view.joinVoiceChannel(channelId);
    },
    [enter],
  );

  useEffect(() => {
    if (pendingJoin === null) return;
    const { channelId } = pendingJoin;
    const ready = callChannel === channelId && callConnected;
    const timer = setTimeout(
      () => {
        setPendingJoin(null);
        if (ready) {
          void enter(channelId);
          return;
        }
        setJoining(null);
        setJoinFailure({ channelId, text: latest.current.t("activities.error.seat") });
      },
      ready ? 0 : Math.max(0, pendingJoin.at + JOIN_WAIT_MS - Date.now()),
    );
    return () => clearTimeout(timer);
  }, [callChannel, callConnected, enter, pendingJoin]);

  const launch = useCallback(
    async (place: string, picked: PickedActivity, options: { replace?: boolean } = {}) => {
      const device = deviceClass();
      // One for yourself at a time: the one running makes way, but only once
      // the next one has started, so a refused start leaves it as it was.
      const running = place === PERSONAL ? latest.current.personal : null;
      const reply = await activitiesApi.launch(place, {
        install: picked.install,
        device,
        ...(options.replace === true ? { replace: true } : {}),
      });
      if (!alive.current) return;
      const session: ActivitySession = {
        place,
        install: reply.install,
        access: reply.access,
        src: reply.src,
        title: reply.title || picked.name,
        icon: picked.icon,
        permissions: reply.permissions,
        embedHosts: reply.embedHosts,
      };
      const { selfId: me, ui: view } = latest.current;
      setPicker(null);
      setEnded(null);
      if (place === PERSONAL) {
        if (running !== null && running.access !== session.access) {
          void activitiesApi.leave(PERSONAL, leaseOf(running)).catch(() => undefined);
        }
        setPersonal(session);
        view.openActivity();
        return;
      }
      setVoiceSession(session);
      // Said by the hub in a moment; drawn now so the bar and the rail do not
      // offer to join what you are already in.
      setLive((current) => {
        const known = current.get(place);
        if (known?.install === reply.install) return current;
        return withLive(current, place, {
          install: reply.install,
          name: session.title,
          icon: picked.icon,
          host: reply.host ?? me,
          participants: [me],
          seats: picked.seats,
        });
      });
      view.returnToChannel();
    },
    [],
  );

  const leave = useCallback((slot: SessionSlot) => {
    const current = slot === "voice" ? latest.current.voiceSession : latest.current.personal;
    if (current === null) return;
    (slot === "voice" ? setVoiceSession : setPersonal)(null);
    if (slot === "personal") latest.current.ui.closeActivity();
    void activitiesApi.leave(current.place, leaseOf(current)).catch(() => undefined);
    if (slot === "voice") {
      const me = latest.current.selfId;
      setLive((map) => {
        const known = map.get(current.place);
        if (known === undefined || !known.participants.includes(me)) return map;
        const participants = known.participants.filter((person) => person !== me);
        return withLive(map, current.place, participants.length === 0 ? null : { ...known, participants });
      });
    }
  }, []);

  const end = useCallback(async (channelId: string) => {
    await activitiesApi.end(channelId);
    if (!alive.current) return;
    setEnded(null);
    setVoiceSession((current) => (current?.place === channelId ? null : current));
    setLive((current) => withLive(current, channelId, null));
  }, []);

  const lose = useCallback((slot: SessionSlot, access: string) => drop(slot, access, true), [drop]);

  const openPicker = useCallback((request: PickerRequest) => {
    // The picker and its questions are drawn over the app, which full screen hides.
    if (document.fullscreenElement) void document.exitFullscreen?.().catch(() => undefined);
    setPicker(request);
  }, []);
  const closePicker = useCallback(() => setPicker(null), []);

  const publishSlot = useMemo(
    () => ({
      voice: (element: HTMLElement | null) =>
        setSlots((current) => (current.voice === element ? current : { ...current, voice: element })),
      personal: (element: HTMLElement | null) =>
        setSlots((current) => (current.personal === element ? current : { ...current, personal: element })),
    }),
    [],
  );
  const publishBar = useMemo(
    () => ({
      start: (element: HTMLElement | null) =>
        setBars((current) => (current.start === element ? current : { ...current, start: element })),
      end: (element: HTMLElement | null) =>
        setBars((current) => (current.end === element ? current : { ...current, end: element })),
    }),
    [],
  );

  const value = useMemo<ActivitiesValue>(
    () => ({
      live,
      voice: voiceSession,
      personal,
      joining,
      joinFailure,
      ended,
      picker,
      openPicker,
      closePicker,
      join,
      launch,
      leave,
      end,
      lose,
      slots,
      publishSlot,
      bars,
      publishBar,
    }),
    [
      live,
      voiceSession,
      personal,
      joining,
      joinFailure,
      ended,
      picker,
      openPicker,
      closePicker,
      join,
      launch,
      leave,
      end,
      lose,
      slots,
      publishSlot,
      bars,
      publishBar,
    ],
  );

  return <ActivitiesContext.Provider value={value}>{children}</ActivitiesContext.Provider>;
}

/**
 * Renews one session's seat: at once, then every `LEASE_RENEW_MS`, and again
 * at once whenever `beat` moves. The renewal is the library request with the
 * lease, so it also brings the channel's summary.
 */
function useLease(
  session: ActivitySession | null,
  beat: number,
  setLive: (update: (current: ReadonlyMap<string, ActivitySummary>) => ReadonlyMap<string, ActivitySummary>) => void,
  drop: (slot: SessionSlot, access: string, announce: boolean) => void,
  slot: SessionSlot,
) {
  useEffect(() => {
    if (session === null) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const renew = async () => {
      try {
        const library = await activitiesApi.list(
          session.place,
          { device: deviceClass(), lease: leaseOf(session) },
          controller.signal,
        );
        if (controller.signal.aborted) return;
        if (session.place !== PERSONAL) setLive((current) => withLive(current, session.place, library.live));
      } catch (cause) {
        if (controller.signal.aborted) return;
        // Gone, or no longer yours: anything else is worth another try.
        if (cause instanceof ActivityRequestError && [403, 404, 410].includes(cause.status)) {
          drop(slot, session.access, true);
          return;
        }
      }
      timer = setTimeout(() => void renew(), LEASE_RENEW_MS);
    };
    void renew();
    return () => {
      controller.abort();
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [session, beat, setLive, drop, slot]);
}

/**
 * The ref a view puts on the element a session's frame should cover. Asked
 * for on its own, so the view's other reads of the activities stay plain values.
 */
export function useActivitySlot(slot: SessionSlot): (element: HTMLElement | null) => void {
  return useActivitiesContext().publishSlot[slot];
}

/** The refs the call's bar puts on the places it keeps for the voice activity's controls. */
export function useActivityBar(): Readonly<Record<BarSlot, (element: HTMLElement | null) => void>> {
  return useActivitiesContext().publishBar;
}

export function useActivitiesContext(): ActivitiesValue {
  const value = useContext(ActivitiesContext);
  if (value === null) {
    throw new Error("useActivities must be used inside <ActivitiesProvider>");
  }
  return value;
}
