import { Icon } from "@stellarr/icons";
import { Link, useParams, useRouter } from "@stellarr/router";
import { useEffect, useRef, useState } from "react";

import controls from "@/components/auth/AuthControls.module.css";
import { AuthFrame } from "@/components/auth/AuthFrame";
import { Face, ServerTile } from "@/components/auth/Faces";
import { ServerUnreachable } from "@/components/auth/Outcomes";
import { AppSkeleton } from "@/components/shell/AppSkeleton";
import { useSignInProviders } from "@/hooks/useInstanceFacts";
import { offerInstallAfterInvite } from "@/lib/platform/install";
import { ApiError } from "@/lib/api/errors";
import { pendingInvite, rememberPendingInvite, takePendingInvite } from "@/lib/auth/invites";
import { formatDate } from "@/lib/format/format";
import type { InvitePreview } from "@/ports";
import { serverMark } from "@/state/directoryView";
import { useLanguage } from "@/state/LanguageProvider";
import { usePorts } from "@/state/PortsProvider";
import { useSessionContext as useSession } from "@/state/SessionProvider";

import { usePrefetchApp } from "./loadApp";

type Preview =
  | { state: "loading" }
  | { state: "ready"; invite: InvitePreview }
  /** Unknown, expired, revoked, used up, or its server is gone: the server does not say which. */
  | { state: "unusable" }
  | { state: "failed" };

/** An invite that leads nowhere, with the one way on from here. */
function InviteUnusable({ signedIn }: { signedIn: boolean }) {
  const { t } = useLanguage();
  return (
    <AuthFrame outcome="failure" title={t("auth.invite.unusableTitle")} description={t("auth.invite.unusable")}>
      <Link className={`${controls.button} ${controls.primary} ${controls.wide}`} href={signedIn ? "/" : "/login"}>
        {signedIn ? t("auth.invite.backToApp") : t("auth.invite.backToLogin")}
      </Link>
    </AuthFrame>
  );
}

/**
 * Shows who invited the visitor, where the link leads, and how to accept it.
 */
function InviteDetails({ code, invite, join, joining }: {
  code: string;
  invite: InvitePreview;
  join?: () => void;
  joining: boolean;
}) {
  const { t, locale } = useLanguage();
  const router = useRouter();
  const providers = useSignInProviders();
  const back = `/invite/${encodeURIComponent(code)}`;
  const go = (path: string) => {
    rememberPendingInvite(code);
    router.push(path);
  };

  return (
    <AuthFrame
      scene="invite"
      title={invite.server.name}
      description={invite.server.description}
      headingAction={
        <div className={controls.invite}>
          <ServerTile mark={serverMark(invite.server.name)} iconUrl={invite.server.iconUrl} large />
          <p className={controls.from}>
            <Face
              name={invite.inviter.displayName}
              color={invite.inviter.avatarColor}
              imageUrl={invite.inviter.avatarUrl}
              small
            />
            <span>{t("auth.invite.from", { inviter: invite.inviter.displayName })}</span>
          </p>
        </div>
      }
    >
      {invite.channel !== null || invite.expiresAt !== null ? (
        <ul className={controls.facts}>
          {invite.channel === null ? null : (
            <li>
              <Icon name="hash" size={15} />
              <span>{invite.channel.name}</span>
            </li>
          )}
          {invite.expiresAt === null ? null : (
            <li>
              <Icon name="clock" size={15} />
              <span>{t("auth.invite.validUntil", { date: formatDate(invite.expiresAt, locale) })}</span>
            </li>
          )}
        </ul>
      ) : null}
      {join ? (
        <button
          type="button"
          className={`${controls.button} ${controls.primary} ${controls.wide}`}
          onClick={join}
          disabled={joining}
        >
          {t("auth.invite.join")}
        </button>
      ) : (
        <>
          <div className={controls.stack}>
            <button
              type="button"
              className={`${controls.button} ${controls.primary} ${controls.wide}`}
              onClick={() => go("/register")}
            >
              {t("auth.invite.register")}
            </button>
            <button
              type="button"
              className={`${controls.button} ${controls.secondary} ${controls.wide}`}
              onClick={() => go("/login")}
            >
              {t("auth.invite.login")}
            </button>
            {providers.map((provider) => (
              // The provider returns the visitor to this invite after sign-in.
              <a
                key={provider.loginUrl}
                className={`${controls.button} ${controls.secondary} ${controls.wide}`}
                href={`${provider.loginUrl}?returnUrl=${encodeURIComponent(back)}`}
                onClick={() => rememberPendingInvite(code)}
              >
                <Icon name="login" size={20} />
                <span>{t("auth.invite.provider", { provider: provider.displayName })}</span>
              </a>
            ))}
          </div>
          <p className={controls.hint}>{t("auth.invite.after", { server: invite.server.name })}</p>
        </>
      )}
    </AuthFrame>
  );
}

/**
 * An invite shows a preview before joining. A signed-in visitor confirms
 * here, and a signed-out visitor can choose how to sign in.
 *
 * The code is the last segment of the address, decoded, as the way back
 * from sign-in encodes it; an address whose code cannot be decoded fails like
 * a code the server refuses, without asking the server.
 */
export default function InvitePage() {
  const { code } = useParams<"code">();
  const router = useRouter();
  const ports = usePorts();
  const { phase, reload } = useSession();
  const [joinStatus, setJoinStatus] = useState<{ code: string; state: "joining" | "failed" } | null>(null);
  const pendingJoin = useRef<AbortController | null>(null);
  const [attempt, setAttempt] = useState(0);
  const asked = `${code ?? ""}#${attempt}`;
  const [answer, setAnswer] = useState<{ asked: string; preview: Preview } | null>(null);
  const preview: Preview = answer?.asked === asked ? answer.preview : { state: "loading" };
  const { t } = useLanguage();
  // Signed in, an invite ends in the app.
  usePrefetchApp(phase !== "loading");

  useEffect(() => () => {
    pendingJoin.current?.abort();
    pendingJoin.current = null;
  }, [code]);

  async function joinInvite() {
    if (code === null || phase !== "ready" || pendingJoin.current !== null) return;
    const current = new AbortController();
    pendingJoin.current = current;
    setJoinStatus({ code, state: "joining" });
    const forget = () => {
      if (pendingInvite() === code) takePendingInvite();
    };
    try {
      const server = await ports.directory.joinInvite(code);
      if (current.signal.aborted) return;
      forget();
      offerInstallAfterInvite();
      ports.directory.selectServer(server.id);
      router.replace("/");
      reload();
    } catch {
      if (current.signal.aborted) return;
      forget();
      setJoinStatus({ code, state: "failed" });
    } finally {
      if (pendingJoin.current === current) pendingJoin.current = null;
    }
  }

  useEffect(() => {
    if (code === null || (phase !== "anonymous" && phase !== "ready")) return;
    let active = true;
    ports.session.invitePreview(code).then(
      (invite) => active && setAnswer({ asked, preview: { state: "ready", invite } }),
      (cause: unknown) => {
        if (!active) return;
        setAnswer({
          asked,
          preview: cause instanceof ApiError && cause.status === 404 ? { state: "unusable" } : { state: "failed" },
        });
      },
    );
    return () => {
      active = false;
    };
  }, [asked, code, phase, ports]);

  if (phase === "error") return <ServerUnreachable t={t} onRetry={reload} />;
  if (code === null) return <InviteUnusable signedIn={phase === "ready"} />;
  if (phase !== "anonymous" && phase !== "ready") return <AppSkeleton />;
  const signedIn = phase === "ready";
  if (joinStatus?.code === code && joinStatus.state === "failed") return <InviteUnusable signedIn />;

  switch (preview.state) {
    case "ready":
      return <InviteDetails
        code={code}
        invite={preview.invite}
        join={signedIn ? () => void joinInvite() : undefined}
        joining={joinStatus?.code === code && joinStatus.state === "joining"}
      />;
    case "unusable":
      return <InviteUnusable signedIn={signedIn} />;
    case "failed":
      return <ServerUnreachable t={t} onRetry={() => setAttempt((count) => count + 1)} />;
    case "loading":
      return (
        <AuthFrame scene="invite" title={t("auth.invite.checkingTitle")} description={t("auth.invite.checking")}>
          {null}
        </AuthFrame>
      );
  }
}
