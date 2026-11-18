import { useRouter, useSearchParams } from "@stellarr/router";
import { useEffect, useState } from "react";

import controls from "@/components/auth/AuthControls.module.css";
import { AuthFrame } from "@/components/auth/AuthFrame";
import { ServerUnreachable } from "@/components/auth/Outcomes";
import { InviteNeeded, ProviderRegisterScreen, RegisterScreen } from "@/components/auth/RegisterScreen";
import { providerSignUpIssue, registrationIssue } from "@/containers/registrationErrors";
import { useInstanceName, useSignInProviders } from "@/hooks/useInstanceFacts";
import { useRedirectWhenSignedIn } from "@/hooks/useRedirectWhenSignedIn";
import { ApiError } from "@/lib/api/errors";
import { PROVIDER_SIGN_UP, pendingInvite, rememberPendingInvite, takePendingInvite } from "@/lib/auth/invites";
import type { AuthResult, PendingIdentity } from "@/ports";
import { useLanguage, usePorts, useSession } from "@/state/hooks";

import { usePrefetchApp } from "./loadApp";

type Invite =
  | { state: "none" }
  | { state: "unusable" }
  | { state: "known"; code: string; server: string; inviter: string }
  /** The preview did not come; the form still works with the code alone. */
  | { state: "unnamed"; code: string };

/** The identity an account through a provider is made from, at `?with=provider`. */
type Waiting =
  | { state: "loading" }
  | { state: "ready"; identity: PendingIdentity }
  | { state: "gone" }
  | { state: "failed" };

/**
 * Registration, which on a set-up instance always comes with an invite: the
 * one this tab remembered from the invite page. The instance's first account
 * is made at /setup, where every address leads until it exists.
 *
 * At `?with=provider` the account is made from the identity a provider just
 * confirmed for this browser, with no password, when the owner lets that
 * provider create accounts. The providers that may are offered on the plain
 * form, each sending the visitor there and back with the invite remembered.
 */
export default function RegistrationPage() {
  const ports = usePorts();
  const router = useRouter();
  const { phase, reload } = useSession();
  const { t } = useLanguage();
  const instanceName = useInstanceName();
  const providers = useSignInProviders();
  const throughProvider = useSearchParams().get("with") === "provider";
  const [invite, setInvite] = useState<Invite>(() => {
    const code = pendingInvite();
    return code === null ? { state: "none" } : { state: "unnamed", code };
  });
  const [waiting, setWaiting] = useState<Waiting>({ state: "loading" });
  const [attempt, setAttempt] = useState(0);
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

  useEffect(() => {
    if (!throughProvider || phase !== "anonymous") return;
    let active = true;
    ports.session.pendingIdentity().then(
      (identity) => active && setWaiting(identity === null ? { state: "gone" } : { state: "ready", identity }),
      () => active && setWaiting({ state: "failed" }),
    );
    return () => {
      active = false;
    };
  }, [attempt, phase, ports, throughProvider]);

  /** When this server requires a second step, the account exists but has no session yet. */
  const finish = (result: AuthResult) => {
    // The sign-in page resumes the setup from the challenge the server keeps
    // for this browser, and takes the invite once it is done.
    if (result.kind === "signed-in") reload();
    else router.replace("/login");
  };

  if (phase === "error") return <ServerUnreachable t={t} onRetry={reload} />;

  if (code === null) {
    return (
      <InviteNeeded
        t={t}
        instanceName={instanceName}
        unusable={invite.state === "unusable"}
        onOpen={(typed) => {
          if (!throughProvider) {
            router.push(`/invite/${encodeURIComponent(typed)}`);
            return;
          }
          // The provider's sign-in is already done; the invite is all that is missing.
          rememberPendingInvite(typed);
          setInvite({ state: "unnamed", code: typed });
        }}
      />
    );
  }

  const named = invite.state === "known" ? { server: invite.server, inviter: invite.inviter } : null;

  if (throughProvider) {
    if (waiting.state === "failed") return <ServerUnreachable t={t} onRetry={() => setAttempt((count) => count + 1)} />;
    if (waiting.state === "gone") {
      return (
        <AuthFrame outcome="failure" title={t("auth.unlinked.goneTitle")} description={t("auth.unlinked.gone")}>
          <button
            type="button"
            className={`${controls.button} ${controls.primary} ${controls.wide}`}
            onClick={() => router.replace("/login")}
          >
            {t("auth.invite.backToLogin")}
          </button>
        </AuthFrame>
      );
    }
    if (waiting.state === "loading") {
      return (
        <AuthFrame scene="register" title={t("auth.unlinked.checkingTitle")} description={t("auth.unlinked.checking")}>
          {null}
        </AuthFrame>
      );
    }
    // A provider the owner does not let create accounts leaves the plain form.
    if (waiting.identity.signUp) {
      const { identity } = waiting;
      return (
        <ProviderRegisterScreen
          t={t}
          identity={identity}
          invite={named}
          onSubmit={async (values) => {
            try {
              finish(await ports.session.register({ ...values, external: true, inviteCode: code }));
              return null;
            } catch (cause) {
              return providerSignUpIssue(cause, t, identity.provider.displayName);
            }
          }}
        />
      );
    }
  }

  return (
    <RegisterScreen
      t={t}
      invite={named}
      providers={providers
        .filter((provider) => provider.signUp)
        .map((provider) => ({
          name: provider.displayName,
          href: `${provider.loginUrl}?returnUrl=${encodeURIComponent(PROVIDER_SIGN_UP)}`,
        }))}
      onSubmit={async (values) => {
        try {
          finish(await ports.session.register({ ...values, inviteCode: code }));
          return null;
        } catch (cause) {
          return registrationIssue(cause, t);
        }
      }}
    />
  );
}
