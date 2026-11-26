import { useRouter, useSearchParams } from "@stellarr/router";
import { lazy, useCallback, Suspense, useEffect, useState } from "react";

import { ErrorNotice, type ErrorNoticeProps } from "@/components/primitives/ErrorNotice";
import { AppSkeleton } from "@/components/shell/AppSkeleton";
import { launchEntry } from "@/lib/platform/launch";
import { useLanguage } from "@/state/LanguageProvider";
import { useSessionContext as useSession } from "@/state/SessionProvider";

import { canPrefetchApp, loadApp } from "./loadApp";

/** Why the page cannot show the app, and the way to recover. */
function LoadFailed({ title, message, actions }: Pick<ErrorNoticeProps, "title" | "message" | "actions">) {
  return (
    <main style={{ height: "100%", display: "grid", placeItems: "center", padding: 24 }}>
      <div style={{ width: "min(420px, 100%)" }}>
        <ErrorNotice title={title} message={message} icon="wifi" actions={actions} />
      </div>
    </main>
  );
}

/**
 * Stands in for the app when its code cannot be loaded: the network failed,
 * or this tab still runs a release the server has replaced, whose files are
 * gone. Loading the page again brings the current release.
 */
function AppUnavailable() {
  const { t } = useLanguage();
  return (
    <LoadFailed
      title={t("app.loadFailed")}
      message={t("app.checkConnection")}
      actions={[{ label: t("common.tryAgain"), onClick: () => window.location.reload() }]}
    />
  );
}

const AppContainer = lazy(() =>
  loadApp().then(
    ({ AppContainer }) => ({ default: AppContainer }),
    () => ({ default: AppUnavailable }),
  ),
);

/**
 * The gate.
 *
 * Only `bootstrap` can tell a valid cookie from a stale one, so the route waits
 * for the session to settle instead of guessing from a cookie that may already
 * have lapsed. An anonymous visitor is sent to /login; a failed boot gets the
 * reason and a way to recover, never a blank page.
 *
 * The signed-in app is the bulk of the client's code, so it loads on its own.
 * The pages that lead here can warm it up while idle. This page starts it
 * alongside the session check on a typical connection, or after a successful
 * check when the browser reports a constrained connection.
 */
export default function HomePage() {
  const router = useRouter();
  const entry = launchEntry(useSearchParams());
  const finishEntry = useCallback(() => router.replace("/"), [router]);
  const { phase, verifiedAt, error, reload, logout } = useSession();
  const { t } = useLanguage();

  const [offline, setOffline] = useState(() => navigator.onLine === false);
  useEffect(() => {
    const disconnected = () => setOffline(true);
    const reconnected = () => {
      setOffline(false);
      if (phase === "error" || phase === "loading") reload();
    };
    window.addEventListener("offline", disconnected);
    window.addEventListener("online", reconnected);
    return () => {
      window.removeEventListener("offline", disconnected);
      window.removeEventListener("online", reconnected);
    };
  }, [phase, reload]);

  useEffect(() => {
    if (phase !== "ready") return;
    const onNotice = (event: MessageEvent) => {
      if (event.data?.type !== "stellarr-open-notice" || typeof event.data.url !== "string") return;
      const entry = launchEntry(new URLSearchParams(event.data.url.slice(2)));
      if (!entry?.startsWith("?notice=") || event.data.url !== "/" + entry) return;
      router.replace("/" + entry);
      event.ports[0]?.postMessage(true);
    };
    navigator.serviceWorker?.addEventListener("message", onNotice);
    return () => navigator.serviceWorker?.removeEventListener("message", onNotice);
  }, [phase, router]);

  const mayWarmApp = verifiedAt != null || canPrefetchApp();
  useEffect(() => {
    // A failure shows once the app is wanted, through `AppContainer`.
    if (mayWarmApp) void loadApp().catch(() => undefined);
  }, [mayWarmApp]);

  useEffect(() => {
    if (phase === "anonymous") router.replace(entry ? "/login?returnUrl=" + encodeURIComponent("/" + entry) : "/login");
  }, [phase, router, entry]);

  if (phase === "ready") {
    return (
      <Suspense fallback={<AppSkeleton verifiedAt={verifiedAt} />}>
        <AppContainer entry={entry} onEntryHandled={finishEntry} />
      </Suspense>
    );
  }

  if (offline && (phase === "error" || phase === "loading")) {
    return <LoadFailed title={t("pwa.offlineTitle")} message={t("pwa.offlineBody")}
      actions={[{ label: t("common.tryAgain"), onClick: reload }]} />;
  }

  if (phase === "error" || phase === "revoked") {
    const revoked = phase === "revoked";
    return (
      <LoadFailed
        title={revoked ? t("app.accessRevoked") : t("app.loadFailed")}
        message={error ?? t("app.checkConnection")}
        actions={
          revoked
            ? [{ label: t("common.logout"), onClick: () => void logout() }]
            : [{ label: t("common.tryAgain"), onClick: reload }]
        }
      />
    );
  }

  // Loading, and the moment between "anonymous" and the route change.
  return <AppSkeleton verifiedAt={phase === "loading" ? verifiedAt : null} />;
}
