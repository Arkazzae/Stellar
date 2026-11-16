import type { ReactNode } from "react";

import { Button } from "@stellarr/ui";
import { Avatar } from "@/components/primitives/Avatar";
import { Switch } from "@/components/primitives/Switch";
import type { UserPreferences } from "@/ports";
import { useLanguage } from "@/state/hooks";

import { VoiceSection } from "./VoiceFields";

/**
 * The switch panes: privacy and advanced.
 *
 * They share one shape, a card of switches each with a sentence saying what it
 * changes, so they share this file and its row. Everything here follows the
 * person to every device: the preferences are the account's, and the GIPHY
 * switch is kept in the privacy section of the client's own settings.
 * Notifications grew out of that shape and have their own file.
 */

export interface PreferencePaneProps {
  preferences: UserPreferences;
  onChange: (patch: Partial<UserPreferences>) => void;
}

/** Someone this account blocked, as the privacy pane lists them. */
export interface BlockedPerson {
  id: string;
  displayName: string;
  username: string;
  avatarUrl?: string;
}

/** The block list, owned by the friends store rather than by the preferences. */
export interface BlockedPeopleProps {
  people: readonly BlockedPerson[];
  onUnblock: (id: string) => void;
}

/**
 * The person's own GIPHY switch. It lives in this browser and follows the
 * account with the client's other settings, not with the preferences above.
 */
export interface GiphyPreferenceProps {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
}

/** Starting with the operating system, offered only by the native client. */
export interface StartupPreferenceProps {
  enabled: boolean;
  pending: boolean;
  error: boolean;
  onChange: (enabled: boolean) => void;
}

interface PreferenceRowProps {
  label: string;
  detail: string;
  /** Why the last change did not take. */
  error?: string | null;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}

function PreferenceRow({ label, detail, error, checked, disabled, onChange }: PreferenceRowProps) {
  return (
    <div className="frm-row">
      <span className="frm-lab">
        {label}
        <span>{detail}</span>
        {error ? <span className="frm-detail-error" role="alert">{error}</span> : null}
      </span>
      <div className="frm-ctl frm-switch">
        <Switch checked={checked} onChange={onChange} label={label} disabled={disabled} />
      </div>
    </div>
  );
}

function SwitchCard({ title, lone, children }: { title: string; lone?: boolean; children: ReactNode }) {
  return (
    <VoiceSection title={title} lone={lone}>
      <div className="frm">{children}</div>
    </VoiceSection>
  );
}

export function PrivacyPane({
  preferences,
  onChange,
  blocked,
  giphy,
}: PreferencePaneProps & {
  blocked?: BlockedPeopleProps;
  /** Absent while the instance offers no GIFs, when the switch would change nothing. */
  giphy?: GiphyPreferenceProps;
}) {
  const { t } = useLanguage();
  return (
    <>
      <SwitchCard title={t("preference.contactVisibility")}>
        {/* The details say what turning a switch off takes away, because the
            off side is the one with consequences. */}
        <PreferenceRow
          label={t("preference.directMessages")}
          detail={t("preference.directMessagesDetail")}
          checked={preferences.allowDirectMessages}
          onChange={(allowDirectMessages) => onChange({ allowDirectMessages })}
        />
        <PreferenceRow
          label={t("preference.friendRequests")}
          detail={t("preference.friendRequestsDetail")}
          checked={preferences.allowFriendRequests}
          onChange={(allowFriendRequests) => onChange({ allowFriendRequests })}
        />
        <PreferenceRow
          label={t("preference.showPresence")}
          detail={t("preference.showPresenceDetail")}
          checked={preferences.showPresence}
          onChange={(showPresence) => onChange({ showPresence })}
        />
      </SwitchCard>
      {giphy === undefined ? null : (
        <SwitchCard title={t("preference.gifs")}>
          {/* Here the on side is the one with consequences, so the detail
              says what turning it on sends, and to whom. */}
          <PreferenceRow
            label={t("preference.giphy")}
            detail={t("preference.giphyDetail")}
            checked={giphy.enabled}
            onChange={giphy.onChange}
          />
        </SwitchCard>
      )}
      {blocked === undefined ? null : <BlockedCard {...blocked} />}
    </>
  );
}

/**
 * The people you blocked. Blocking happens where the person is, on their card
 * or in a menu; this is where you find them again once they are out of sight.
 */
function BlockedCard({ people, onUnblock }: BlockedPeopleProps) {
  const { t } = useLanguage();
  return (
    <section className="sc acc-security">
      <header className="sc-h">
        {t("preference.blocked")}
      </header>
      <div className="sc-b acc-security-list">
        {people.length === 0 ? (
          <p className="acc-security-item acc-security-empty">{t("preference.blockedEmpty")}</p>
        ) : (
          people.map((person) => (
            <div className="acc-security-item" key={person.id}>
              <div className="acc-security-summary acc-blocked">
                <Avatar
                  id={person.id}
                  displayName={person.displayName || person.username}
                  {...(person.avatarUrl ? { avatarUrl: person.avatarUrl } : {})}
                  size="sm"
                />
                <span className="acc-security-copy">
                  <b>{person.displayName || person.username}</b>
                  <small>@{person.username}</small>
                </span>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  aria-label={t("preference.unblockPerson", {
                    name: person.displayName || person.username,
                  })}
                  onClick={() => onUnblock(person.id)}
                >
                  {t("common.unblock")}
                </Button>
              </div>
            </div>
          ))
        )}
      </div>
    </section>
  );
}

export function AdvancedPane({
  preferences,
  onChange,
  startup,
}: PreferencePaneProps & { startup?: StartupPreferenceProps }) {
  const { t } = useLanguage();
  return (
    <>
      {startup === undefined ? null : (
        // Autostart lives in the operating system, not in the stored
        // preferences.
        <SwitchCard title={t("preference.system")}>
          <PreferenceRow
            label={t("preference.startup")}
            detail={t("preference.startupDetail")}
            error={startup.error ? t("preference.startupFailed") : null}
            checked={startup.enabled}
            disabled={startup.pending}
            onChange={startup.onChange}
          />
        </SwitchCard>
      )}
      {/* Reduced motion used to live here; it is a question of how the app
          looks and reads, so it sits with contrast and text size now. */}
      <SwitchCard title={t("preference.advanced")} lone={startup === undefined}>
        <PreferenceRow
          label={t("preference.developerMode")}
          detail={t("preference.developerModeDetail")}
          checked={preferences.developerMode}
          onChange={(developerMode) => onChange({ developerMode })}
        />
      </SwitchCard>
    </>
  );
}
