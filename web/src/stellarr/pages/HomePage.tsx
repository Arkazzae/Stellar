"use client";

import { useRouter } from "@stellarr/router";
import { lazy, Suspense, useEffect } from "react";

import { ErrorNotice } from "@/components/primitives/ErrorNotice";
import { AppSkeleton } from "@/components/shell/AppSkeleton";
import { useLanguage, useSession } from "@/state/hooks";

const loadApp = () => import("@/containers/AppContainer");
const AppContainer = lazy(() => loadApp().then(({ AppContainer }) => ({ default: AppContainer })));

/**
 * The gate.
 *
 * Only `bootstrap` can tell a valid cookie from a stale one, so the route waits
 * for the session to settle instead of guessing from a cookie that may already
 * have lapsed. An anonymous visitor is sent to /login; a failed boot gets the
 * reason and a way to recover, never a blank page.
 *
 * The signed-in app is the bulk of the client's code, so it loads on its own.
 * It starts loading as soon as this page opens, alongside the session check,
 * and the skeleton stands in until both are done; the sign-in pages never
 * wait for it.
 */
export default function HomePage() {
  const router = useRouter();
  const { phase, error, reload, logout } = useSession();
  const { t } = useLanguage();

  useEffect(() => {
    void loadApp();
  }, []);

  useEffect(() => {
    if (phase === "anonymous") router.replace("/login");
  }, [phase, router]);

  if (phase === "ready") {
    return (
      <Suspense fallback={<AppSkeleton />}>
        <AppContainer />
      </Suspense>
    );
  }

  if (phase === "error" || phase === "revoked") {
    const revoked = phase === "revoked";
    return (
      <main style={{ height: "100%", display: "grid", placeItems: "center", padding: 24 }}>
        <div style={{ width: "min(420px, 100%)" }}>
          <ErrorNotice
            title={revoked ? t("app.accessRevoked") : t("app.loadFailed")}
            message={error ?? t("app.checkConnection")}
            icon="wifi"
            actions={
              revoked
                ? [{ label: t("common.logout"), onClick: () => void logout() }]
                : [{ label: t("common.tryAgain"), onClick: reload }]
            }
          />
        </div>
      </main>
    );
  }

  // Loading, and the moment between "anonymous" and the route change.
  return <AppSkeleton />;
}
