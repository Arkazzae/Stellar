import { Button } from "@stellarr/ui";
import { Avatar } from "@/components/primitives/Avatar";
import { formatShortDate } from "@/components/settings/paneFormat";
import { SettingsSection } from "@/components/settings/SettingsSection";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import type { Ban } from "@/ports";
import { useLanguage } from "@/state/hooks";

export interface BansPaneProps {
  supported: boolean;
  bans: readonly Ban[];
  onUnban: (userId: string) => Promise<void>;
}

export function BansPane({ supported, bans, onUnban }: BansPaneProps) {
  const { locale, t } = useLanguage();
  const unban = useAsyncAction();

  return (
    <>
      {supported ? null : <p className="set-note">{t("settings.bansUnsupported")}</p>}
      <SettingsSection title={t("srv.bans.title")} count={bans.length} bodyClassName="sec-list ss-list">
        {unban.failed ? (
          <p className="set-note sound-error" role="alert">
            {t("settings.unbanFailed")}
          </p>
        ) : null}
        {bans.length === 0 ? (
          <p className="acc-security-item acc-security-empty">{t("srv.bans.empty")}</p>
        ) : (
          bans.map((ban) => (
            <div className="sec-row" key={ban.userId}>
              {/* Banned accounts are drained of hue, like anyone offline. */}
              <Avatar id={ban.userId} displayName={ban.displayName} size="sm" offline />
              <span className="sec-copy">
                <b>{ban.displayName}</b>
                <small>
                  {t("srv.bans.meta", {
                    name: ban.bannedByName,
                    date: formatShortDate(ban.bannedAt, locale),
                    reason: ban.reason,
                  })}
                </small>
              </span>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={!supported || unban.pending}
                aria-label={t("srv.bans.unbanNamed", { name: ban.displayName })}
                onClick={() => void unban.run(() => onUnban(ban.userId))}
              >
                {t("srv.bans.unban")}
              </Button>
            </div>
          ))
        )}
      </SettingsSection>
    </>
  );
}
