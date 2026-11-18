import { useId } from "react";

import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import type { ActiveFriendVM, OpenProfile } from "@/components/view-models";
import type { Translator } from "@/i18n";
import { PRESENCE_LABEL } from "@/ports";
import { useLanguage } from "@/state/LanguageProvider";

export interface FriendsActivityProps {
  friends: ActiveFriendVM[];
  onOpenProfile: OpenProfile;
  onMessage: (userId: string) => void;
}

/**
 * The one line under a name. Being in a call is the activity this panel is
 * for, so it wins over what the person wrote about themselves, which wins over
 * the bare presence word.
 */
function activityOf(
  { person, customStatus, inCall, voiceChannel }: ActiveFriendVM,
  t: Translator,
): string {
  if (voiceChannel) {
    return t("friends.activityInChannel", {
      channel: voiceChannel.name,
      server: voiceChannel.serverName,
    });
  }
  if (inCall) return t("profile.inCall");
  return customStatus ?? t(PRESENCE_LABEL[person.presence]);
}

/**
 * The live activity list beside Friends; it never appears beside a DM.
 *
 * Built like the roster, header and rows alike, because it is the same thing
 * from the other side: who is around right now.
 */
export function FriendsActivity({ friends, onOpenProfile, onMessage }: FriendsActivityProps) {
  const { t } = useLanguage();
  const titleId = useId();

  return (
    <aside className="fr-activity" aria-labelledby={titleId}>
      <div className="roster-head">
        <h3 id={titleId}>{t("friends.activeNow")}</h3>
      </div>

      <div className="roster-list">
        {friends.length === 0 ? (
          <p className="fr-activity-empty">{t("friends.activityEmpty")}</p>
        ) : (
          <ul className="person-list">
            {friends.map((entry) => {
              const { person } = entry;
              const activity = activityOf(entry, t);
              const activityId = `${titleId}-${person.id}`;
              return (
                <li className="person has-menu" key={person.id}>
                  <button
                    type="button"
                    className="person-main"
                    aria-haspopup="dialog"
                    aria-label={t("profile.label", { name: person.displayName })}
                    aria-describedby={activityId}
                    onClick={(event) => onOpenProfile(person.id, event.currentTarget)}
                  >
                    <Avatar
                      id={person.id}
                      displayName={person.displayName}
                      initials={person.initials}
                      avatarUrl={person.avatarUrl}
                      presence={person.presence}
                    />
                    <span className="who">
                      <b>{person.displayName}</b>
                      <span id={activityId} className="person-sub" title={activity}>
                        {activity}
                      </span>
                    </span>
                  </button>
                  <button
                    type="button"
                    className="person-more"
                    title={t("profile.writeTo", { name: person.displayName })}
                    aria-label={t("profile.writeTo", { name: person.displayName })}
                    onClick={() => onMessage(person.id)}
                  >
                    <Icon name="thread" size={16} />
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </aside>
  );
}
