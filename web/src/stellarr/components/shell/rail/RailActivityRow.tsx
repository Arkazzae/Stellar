import { memo } from "react";

import { ActivityIcon } from "@/components/activities/ActivityIcon";
import type { VoiceChannelActivityVM } from "@/components/view-models";
import { useLanguage } from "@/state/hooks";

export interface RailActivityRowProps {
  channelId: string;
  activity: VoiceChannelActivityVM;
  /** Joins it, joining the channel first when you are not in it. */
  onJoin: (channelId: string) => void;
  /** Back to it on the stage, once you are inside. */
  onOpen: (channelId: string) => void;
}

/**
 * What runs on a voice channel, under its row and at the depth of its people:
 * the activity, how many are inside, and one click to join it. Inside, the row
 * is the current place and takes you back to it; full, it says so instead of
 * a button that would refuse.
 */
export const RailActivityRow = memo(function RailActivityRow({
  channelId,
  activity,
  onJoin,
  onOpen,
}: RailActivityRowProps) {
  const { t } = useLanguage();
  const { name, inside, joining, full, failure } = activity;
  const count = activity.participants.length;
  const content = (
    <>
      <ActivityIcon install={activity.install} name={name} icon={activity.icon} size={18} />
      <span className="rail-act-name">{name}</span>
      <span className="rail-act-count">
        <span aria-hidden="true">{count}</span>
        <span className="sr-only">{t("activities.inside", { count })}</span>
      </span>
    </>
  );

  return (
    <>
      <div className={inside ? "rail-act on" : "rail-act"}>
        {inside ? (
          <button type="button" className="rail-act-main" aria-current="true" onClick={() => onOpen(channelId)}>
            {content}
          </button>
        ) : (
          <span className="rail-act-main">{content}</span>
        )}
        {inside ? null : full ? (
          <span
            className="rail-act-full"
            title={t("activities.fullDetail", { count, seats: activity.seats ?? count })}
          >
            {t("activities.full")}
          </span>
        ) : (
          <button
            type="button"
            className="rail-act-join"
            aria-label={joining ? t("activities.joining") : t("activities.joinNamed", { name })}
            aria-busy={joining || undefined}
            onClick={joining ? undefined : () => onJoin(channelId)}
          >
            {joining ? t("activities.joining") : t("activities.join")}
          </button>
        )}
      </div>
      {failure === null ? null : (
        <p className="rail-act-error" role="alert">
          {failure}
        </p>
      )}
    </>
  );
});
