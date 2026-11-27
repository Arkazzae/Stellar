import { useState } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { SettingsSection } from "@/components/settings/SettingsSection";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import type { InterfaceLocale, Translator } from "@/i18n";
import { inviteExpiry, localInviteCode } from "@/lib/auth/invites";
import type { Invite } from "@/ports";
import { useLanguage } from "@/state/hooks";

/** "never expires", "expired", "expires in 6 days": counts in the coarsest unit that fits. */
function expiry(expiresAt: string | null, now: number, t: Translator, locale: InterfaceLocale): string {
  const left = inviteExpiry(expiresAt, now);
  if (left.state === "never") return t("settings.expiresNever");
  if (left.state === "expired") return t("settings.expired");
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: "always", style: "short" });
  return t("settings.expires", { time: relative.format(left.value, left.unit) });
}

/** The part of the link people tell apart: `invite/abc123`, without the host. */
function shortLink(url: string): string {
  const at = url.indexOf("/invite/");
  return at === -1 ? url : url.slice(at + 1);
}

export interface InvitesPaneProps {
  supported: boolean;
  invites: readonly Invite[];
  onCreate: () => Promise<void>;
  onRevoke: (inviteId: string) => Promise<void>;
}

export function InvitesPane({ supported, invites, onCreate, onRevoke }: InvitesPaneProps) {
  const { locale, t } = useLanguage();
  // One clock read per mount rather than per render, so every row's countdown
  // agrees with the others. The pane is remounted each time it is opened.
  const [now] = useState(() => Date.now());
  const [copiedInviteId, setCopiedInviteId] = useState<string | null>(null);
  const change = useAsyncAction();

  const copyInvite = async (invite: Invite, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedInviteId(invite.id);
    } catch {
      setCopiedInviteId(null);
    }
  };

  return (
    <>
      {supported ? null : <p className="set-note">{t("settings.unsupported")}</p>}
      <SettingsSection
        title={t("srv.invites.title")}
        count={invites.length}
        countLabel={t("settings.invitesCount", { count: invites.length })}
        bodyClassName="sec-list ss-list"
      >
        {change.failed ? (
          <p className="set-note sound-error" role="alert">
            {t("settings.inviteActionFailed")}
          </p>
        ) : null}
        {invites.length === 0 ? (
          <p className="acc-security-item acc-security-empty">{t("srv.invites.empty")}</p>
        ) : (
          invites.map((invite) => {
            const code = localInviteCode(invite.url);
            return (
              <div className="sec-row ss-invite" key={invite.id}>
                <span className="sec-copy">
                  <b className="ss-mono" title={code ?? invite.url}>
                    {code ?? shortLink(invite.url)}
                  </b>
                  <small>
                    {t("srv.invites.meta", {
                      name: invite.createdByName,
                      uses:
                        invite.maxUses === null
                          ? t("srv.invites.usesUnlimited", { used: invite.uses })
                          : t("srv.invites.uses", { used: invite.uses, max: invite.maxUses }),
                      expiry: expiry(invite.expiresAt, now, t, locale),
                    })}
                  </small>
                </span>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  aria-label={t("settings.copyInvite", { url: code ?? invite.url })}
                  onClick={() => void copyInvite(invite, code ?? invite.url)}
                >
                  <Icon name={copiedInviteId === invite.id ? "tick" : "copy"} size={14} />
                  {copiedInviteId === invite.id ? t("invite.copied") : t(code ? "invite.copyCode" : "srv.invites.copy")}
                </Button>
                <Button
                  type="button"
                  variant="danger"
                  size="sm"
                  disabled={!supported || change.pending}
                  aria-label={t("settings.revokeInvite", { url: code ?? invite.url })}
                  onClick={() => void change.run(() => onRevoke(invite.id))}
                >
                  {t("settings.revoke")}
                </Button>
              </div>
            );
          })
        )}
        <div className="sec-row ss-foot">
          <span className="ss-foot-note">{t("srv.invites.note")}</span>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={!supported || change.pending}
            onClick={() => void change.run(onCreate)}
          >
            <Icon name="plus" size={15} />
            {t("settings.newInvite")}
          </Button>
        </div>
      </SettingsSection>
    </>
  );
}
