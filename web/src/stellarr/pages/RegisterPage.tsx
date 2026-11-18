import { useRouter } from "@stellarr/router";
import { useEffect, useState } from "react";

import { ServerUnreachable } from "@/components/auth/Outcomes";
import { InviteNeeded, RegisterScreen } from "@/components/auth/RegisterScreen";
import { registrationIssue } from "@/containers/registrationErrors";
import { useInstanceName } from "@/hooks/useInstanceFacts";
import { useRedirectWhenSignedIn } from "@/hooks/useRedirectWhenSignedIn";
import { ApiError } from "@/lib/api/errors";
import { pendingInvite, takePendingInvite } from "@/lib/auth/invites";
import { useLanguage, usePorts, useSession } from "@/state/hooks";

import { usePrefetchApp } from "./loadApp";

type Invite =
  | { state: "none" }
  | { state: "unusable" }
  | { state: "known"; code: string; server: string; inviter: string }
  /** The preview did not come; the form still works with the code alone. */
  | { state: "unnamed"; code: string };

/**
 * Registration, which on a set-up instance always comes with an invite: the
 * one this tab remembered from the invite page. The instance's first account
 * is made at /setup, where every address leads until it exists.
 */
export default function RegistrationPage() {
  const ports = usePorts();
  const router = useRouter();
  const { phase, reload } = useSession();
  const { t } = useLanguage();
  const instanceName = useInstanceName();
  const [invite, setInvite] = useState<Invite>(() => {
    const code = pendingInvite();
    return code === null ? { state: "none" } : { state: "unnamed", code };
  });
  const code = invite.state === "known" || invite.state === "unnamed" ? invite.code : null;
  usePrefetchApp(phase !== "loading");
  useRedirectWhenSignedIn(phase);

  useEffect(() => {
    if (code === null) return;
    let active = true;
    ports.session.invitePreview(code).then(
      (preview) =>
        active && setInvite({ state: "known", code, server: preview.server.name, inviter: preview.inviter.displayName }),
      (cause: unknown) => {
        if (!active || !(cause instanceof ApiError) || cause.status !== 404) return;
        takePendingInvite();
        setInvite({ state: "unusable" });
      },
    );
    return () => {
      active = false;
    };
  }, [code, ports]);

  if (phase === "error") return <ServerUnreachable t={t} onRetry={reload} />;

  if (code === null) {
    return (
      <InviteNeeded
        t={t}
        instanceName={instanceName}
        unusable={invite.state === "unusable"}
        onOpen={(typed) => router.push(`/invite/${encodeURIComponent(typed)}`)}
      />
    );
  }

  return (
    <RegisterScreen
      t={t}
      invite={invite.state === "known" ? { server: invite.server, inviter: invite.inviter } : null}
      onSubmit={async (values) => {
        try {
          const result = await ports.session.register({ ...values, inviteCode: code });
          // When this server requires a second step, the account exists but
          // has no session yet. The sign-in page resumes the setup from the
          // challenge the server keeps for this browser, and takes the invite
          // once it is done.
          if (result.kind === "signed-in") reload();
          else router.replace("/login");
          return null;
        } catch (cause) {
          return registrationIssue(cause, t);
        }
      }}
    />
  );
}
