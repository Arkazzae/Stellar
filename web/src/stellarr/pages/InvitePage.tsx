import { useParams, useRouter } from "@stellarr/router";
import { useEffect, useState } from "react";

import { ErrorNotice } from "@/components/primitives/ErrorNotice";
import { AppSkeleton } from "@/components/shell/AppSkeleton";
import { rememberPendingInvite } from "@/lib/auth/invites";
import { useLanguage, usePorts, useSession } from "@/state/hooks";

/**
 * Joins the server an invite link names. The code is the last segment of the
 * address, decoded, as the way back from sign-in encodes it; an address whose
 * code cannot be decoded fails like a code the server refuses, without asking
 * the server.
 */
export default function InvitePage() {
  const { code } = useParams<"code">();
  const router = useRouter();
  const ports = usePorts();
  const { phase, reload } = useSession();
  const { t } = useLanguage();
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (code === null) return;
    if (phase === "anonymous") {
      // Registering comes first; the redirect after it brings the visitor back.
      rememberPendingInvite(code);
      router.replace("/register");
      return;
    }
    if (phase !== "ready") return;

    let active = true;
    ports.directory.joinInvite(code).then(
      (server) => {
        if (!active) return;
        ports.directory.selectServer(server.id);
        router.replace("/");
        reload();
      },
      () => {
        if (active) setFailed(true);
      },
    );
    return () => {
      active = false;
    };
  }, [code, phase, ports, reload, router]);

  if (!failed && code !== null) return <AppSkeleton />;

  return (
    <main style={{ height: "100%", display: "grid", placeItems: "center", padding: 24 }}>
      <div style={{ width: "min(420px, 100%)" }}>
        <ErrorNotice
          title={t("shell.inviteInvalid")}
          message={t("shell.inviteInvalidDetail")}
          icon="link"
          actions={[{ label: t("common.close"), onClick: () => router.replace("/") }]}
        />
      </div>
    </main>
  );
}
