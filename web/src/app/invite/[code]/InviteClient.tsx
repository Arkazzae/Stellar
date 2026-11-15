"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState } from "react";

import { ErrorNotice } from "@/components/primitives/ErrorNotice";
import { inviteCodeFromPath, rememberPendingInvite } from "@/lib/invites";
import { useLanguage, usePorts, useSession } from "@/state/hooks";

import { AppSkeleton } from "../../AppSkeleton";

/**
 * Joins the server an invite link names. The code is read from the address,
 * since one page is served for every code.
 */
export function InviteClient() {
  const code = inviteCodeFromPath(usePathname());
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

  // An address with no readable code fails like a code the server refuses.
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
