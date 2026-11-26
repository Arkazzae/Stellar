import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import type { MessageKey, Translator } from "@/i18n";
import { dismissActivityNotifications } from "@/lib/platform/badge";
import { clearAll as clearTranslations } from "@/lib/chatTranslation/translationCache";
import { wipeAll as wipeStoredMessages, wipeServer as wipeStoredServer } from "@/lib/messageStore/messageStore";
import type { Account, AuthResult, ConnectionStatus, Presence, User } from "@/ports";

import { useLanguage } from "./LanguageProvider";
import { usePorts, useRuntime } from "./PortsProvider";

/**
 * `anonymous` is a phase of its own rather than an error: a 401 is the answer
 * to "is anyone signed in", and it leads to the login screen, not to a retry.
 */
export type SessionPhase = "loading" | "ready" | "error" | "anonymous" | "revoked";

export interface SessionValue {
  phase: SessionPhase;
  /** When bootstrap confirmed this session, using the local performance clock. */
  verifiedAt: number | null;
  user: User | null;
  /** The boot failure, already translated. Null unless the phase is `error` or `revoked`. */
  error: string | null;
  /** Control-plane health, straight off `ConnectionPort`. */
  connection: ConnectionStatus;
  /**
   * The status you chose for yourself, which your own row and account card
   * show. It starts as "online": a signed-in tab with a socket is online, and
   * nobody has to say so.
   */
  presence: Presence;
  setPresence: (presence: Presence) => void;
  /** Retries the bootstrap. */
  reload: () => void;
  /**
   * The first step of a sign-in. The app boots only when that was enough;
   * a challenge or a pending recovery is handed back for the sign-in page to
   * finish, and null means the attempt failed and `loginError` says why.
   * `linkPending` asks to link the identity a provider confirmed for this
   * browser once the sign-in is complete.
   */
  login: (
    username: string,
    password: string,
    keepSignedIn?: boolean,
    linkPending?: boolean,
  ) => Promise<AuthResult | null>;
  loginPending: boolean;
  loginError: string | null;
  logout: () => Promise<void>;
  /**
   * Whether the session has ended by now: signed out here, ended by the
   * server, or shut out of the open server. For what runs as the signed-in
   * app goes away, which a reload does too, with the session alive.
   */
  hasEnded: () => boolean;
}

const SessionContext = createContext<SessionValue | null>(null);

/**
 * What a failed request looks like, read structurally.
 *
 * `@/lib/api` is a transport and stays below the adapters, so the error class
 * is not imported to `instanceof` against; the two fields it carries are read
 * instead.
 */
interface RequestFailure {
  kind?: unknown;
  status?: unknown;
}

function failureOf(cause: unknown): RequestFailure {
  return typeof cause === "object" && cause !== null ? (cause as RequestFailure) : {};
}

const isUnauthorized = (cause: unknown): boolean => {
  const { kind, status } = failureOf(cause);
  return kind === "unauthorized" || status === 401;
};

const isForbidden = (cause: unknown): boolean => {
  const { kind, status } = failureOf(cause);
  return kind === "forbidden" || status === 403;
};

const FAILURE_MESSAGE: Partial<Record<string, MessageKey>> = {
  network: "session.network",
  unauthorized: "session.unauthorized",
  forbidden: "session.forbidden",
  "not-found": "session.notFound",
  invalid: "session.invalid",
  "rate-limited": "session.rateLimited",
  malformed: "session.malformed",
  server: "session.server",
};

/** The message for the failure's kind, or `fallback` when it has none. */
function messageFor(cause: unknown, t: Translator, fallback: MessageKey): string {
  const { kind } = failureOf(cause);
  const key = typeof kind === "string" ? FAILURE_MESSAGE[kind] : undefined;
  return t(key ?? fallback);
}

/** Your profile as the account holds it, laid over the identity the boot returned. */
function withProfileOf(user: User, account: Account): User {
  const { avatarUrl: _avatar, bannerUrl: _banner, ...rest } = user;
  return {
    ...rest,
    displayName: account.displayName,
    customStatus: account.customStatus,
    bio: account.bio,
    pronouns: account.pronouns,
    ...(account.avatarUrl === undefined ? {} : { avatarUrl: account.avatarUrl }),
    ...(account.bannerUrl === undefined ? {} : { bannerUrl: account.bannerUrl }),
  };
}

interface Boot {
  phase: SessionPhase;
  verifiedAt?: number;
  user: User | null;
  error: string | null;
}

const LOADING: Boot = { phase: "loading", user: null, error: null };
const ANONYMOUS: Boot = { phase: "anonymous", user: null, error: null };

export function SessionProvider({ children }: { children: ReactNode }) {
  const runtime = useRuntime();
  const ports = usePorts();
  const { t } = useLanguage();
  const [boot, setBoot] = useState<Boot>(LOADING);
  const [reloadToken, reload] = useReducer((count: number) => count + 1, 0);
  const [loginPending, setLoginPending] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [presence, setPresence] = useState<Presence>("online");

  // Kept beside the phase rather than read from it: what goes away with the
  // app asks on its way out, when the phase it last rendered with still says
  // `ready`.
  const ended = useRef(false);
  const hasEnded = useCallback(() => ended.current, []);

  const connection = useSyncExternalStore(
    ports.connection.subscribe,
    ports.connection.getStatus,
    (): ConnectionStatus => "idle",
  );

  const realtimeSessionEnded = useSyncExternalStore(
    ports.connection.subscribe,
    () => runtime.realtime.hasSessionEnded?.() ?? false,
    () => false,
  );

  // A retry, or a swapped-out runtime, puts the session back to `loading`
  // before the bootstrap below runs again. The reset is derived from the inputs
  // during render, so the spinner lands in the same commit as the retry.
  const [lastRun, setLastRun] = useState({ ports, runtime, reloadToken });
  if (
    lastRun.ports !== ports ||
    lastRun.runtime !== runtime ||
    lastRun.reloadToken !== reloadToken
  ) {
    setLastRun({ ports, runtime, reloadToken });
    if (boot !== LOADING) setBoot(LOADING);
  }

  useEffect(() => {
    let active = true;

    async function bootstrap(): Promise<void> {
      try {
        const payload = await ports.session.bootstrap();
        if (!active) return;
        const verifiedAt = performance.now();
        setBoot({ ...LOADING, verifiedAt });
        // Hydrating before the phase flips means the first `ready` render
        // already has channels and members instead of an empty rail.
        await runtime.hydrate(payload);
        if (!active) return;
        // The bootstrap precedes this tab's hub connection, so "offline" means
        // "not connected yet", not a deliberate invisible choice. Any other
        // status was advertised by another tab and is safe to inherit.
        setPresence(payload.user.presence === "offline" ? "online" : payload.user.presence);
        ended.current = false;
        setBoot({
          phase: "ready",
          verifiedAt,
          user: withProfileOf(payload.user, ports.account.getAccount()),
          error: null,
        });
      } catch (cause) {
        if (!active) return;
        if (isUnauthorized(cause)) setBoot(ANONYMOUS);
        else if (isForbidden(cause)) {
          setBoot({ phase: "revoked", user: null, error: t("session.accessRevoked") });
        } else {
          setBoot({
            phase: "error",
            user: null,
            error: messageFor(cause, t, "session.loadFailed"),
          });
        }
      }
    }

    void bootstrap();
    return () => {
      active = false;
    };
  }, [ports, runtime, reloadToken, t]);

  useEffect(
    () =>
      runtime.realtime.on("ServerAccessRevoked", ({ serverId, kind }) => {
        // Whatever server it is, what this device kept of it is no longer theirs to read.
        void wipeStoredServer(serverId);
        if (ports.directory.getServer().id !== serverId) return;
        ended.current = true;
        setBoot({
          phase: "revoked",
          user: null,
          error: t(kind === "banned" ? "session.banned" : "session.kicked"),
        });
      }),
    [ports, runtime, t],
  );

  // A profile edit reaches everything that reads the session identity without
  // a reload. The directory store gets the same change through its own path.
  useEffect(
    () =>
      ports.account.onChanged(() => {
        const account = ports.account.getAccount();
        setBoot((current) =>
          current.user === null || current.user.id !== account.id
            ? current
            : { ...current, user: withProfileOf(current.user, account) },
        );
      }),
    [ports],
  );

  // The socket is strictly downstream of a bootstrap that returned a user.
  // Negotiating in parallel would have an anonymous visitor fire a request that
  // can only answer 401, racing the bootstrap that is about to answer 401 too.
  const signedIn = boot.phase === "ready" && boot.user !== null;

  useEffect(() => {
    if (!signedIn) return;
    // A failed start is not fatal: it shows through `connection`.
    void runtime.realtime.start().catch(() => undefined);
    return () => {
      // `stop` invalidates the old socket before a later `start` can open another.
      void runtime.realtime.stop().catch(() => undefined);
    };
  }, [runtime, signedIn]);

  // Another tab can change this account's status. It arrives as the same event
  // as everyone else's, so every account row reads back the advertised value.
  const userId = boot.user?.id;
  useEffect(() => {
    if (userId === undefined) return;
    return ports.presence.onPresenceChanged((changedId, next) => {
      if (changedId === userId) setPresence(next);
    });
  }, [ports, userId]);

  // The chosen status is asserted again after every reconnect. The hub echoes
  // it to every client, this one included, so the roster and the account row
  // converge on the value the backend owns.
  useEffect(() => {
    if (!signedIn || connection !== "connected") return;
    void ports.presence.setPresence(presence).catch(() => undefined);
  }, [ports, signedIn, connection, presence]);

  const login = useCallback(
    async (username: string, password: string, keepSignedIn = true, linkPending = false) => {
      setLoginPending(true);
      setLoginError(null);
      try {
        const result = await ports.session.login({
          username,
          password,
          ...(keepSignedIn ? {} : { keepSignedIn: false }),
          ...(linkPending ? { linkPending: true } : {}),
        });
        // A second factor is still due: nothing may boot, open the hub or
        // join voice until the challenge is answered.
        if (result.kind === "signed-in") reload();
        return result;
      } catch (cause) {
        setLoginError(
          isUnauthorized(cause)
            ? t("session.loginInvalid")
            : messageFor(cause, t, "session.loginFailed"),
        );
        return null;
      } finally {
        setLoginPending(false);
      }
    },
    [ports, t],
  );

  // The hub can explicitly end this session. Browsers hide the status of a
  // failed WebSocket upgrade, so an ordinary disconnect still needs the HTTP
  // check to tell an unreachable instance from a signed-out account.
  useEffect(() => {
    if (!signedIn) return;

    const endSession = () => {
      ended.current = true;
      // Signed out elsewhere: the keys of a session that ended go too.
      void ports.encryption.forget().catch(() => undefined);
      void clearTranslations().catch(() => undefined);
      void wipeStoredMessages().catch(() => undefined);
      void runtime.realtime.stop().catch(() => undefined);
      setLoginError(t("session.unauthorized"));
      setBoot(ANONYMOUS);
    };

    if (realtimeSessionEnded) {
      endSession();
      return;
    }
    if (connection !== "reconnecting" && connection !== "disconnected") return;

    let active = true;
    ports.session.check().then(
      (alive) => {
        if (active && !alive) endSession();
      },
      () => undefined,
    );
    return () => {
      active = false;
    };
  }, [connection, ports, realtimeSessionEnded, runtime, signedIn, t]);

  // A revoked or expired session cannot be used to remove its push row, but
  // the browser must still stop holding a subscription for it. The server's
  // session binding is what decides delivery; this only frees the device.
  useEffect(() => {
    if (boot.phase !== "anonymous" && boot.phase !== "revoked") return;
    void navigator.clearAppBadge?.().catch(() => undefined);
    void dismissActivityNotifications().catch(() => undefined);
    void ports.notificationDelivery.releaseDevice().catch(() => undefined);
  }, [boot.phase, ports]);

  const logout = useCallback(async () => {
    try {
      // Before the session goes: the server can still recognise the endpoint
      // and forget its row. The local unsubscribe happens inside regardless.
      await ports.notificationDelivery.releaseDevice().catch(() => undefined);
      // This browser's encryption keys go with the session; the server stops
      // counting its device on its own once the session ends.
      await ports.encryption.forget().catch(() => undefined);
      // Stored translations can hold the text of this person's private messages.
      await clearTranslations().catch(() => undefined);
      await wipeStoredMessages().catch(() => undefined);
      await ports.session.logout();
    } finally {
      ended.current = true;
      // The socket goes down even when the request failed: the cookie may
      // already be gone, and an open hub would keep pushing into a signed-out tab.
      await runtime.realtime.stop().catch(() => undefined);
      setPresence("online");
      setBoot(ANONYMOUS);
    }
  }, [ports, runtime]);

  const value = useMemo<SessionValue>(
    () => ({
      phase: boot.phase,
      verifiedAt: boot.verifiedAt ?? null,
      user: boot.user,
      error: boot.error,
      connection,
      presence,
      setPresence,
      reload,
      login,
      loginPending,
      loginError,
      logout,
      hasEnded,
    }),
    [boot, connection, presence, login, loginPending, loginError, logout, hasEnded],
  );

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSessionContext(): SessionValue {
  const value = useContext(SessionContext);
  if (value === null) {
    throw new Error("useSession must be used inside <SessionProvider>");
  }
  return value;
}
