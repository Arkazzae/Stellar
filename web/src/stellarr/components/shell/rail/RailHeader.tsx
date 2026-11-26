import { Icon } from "@stellarr/icons";
import { useLanguage } from "@/state/hooks";

export interface RailHeaderProps {
  serverName: string;
  onCreateChannel?: () => void;
  /** Absent hides the button: who may edit is decided by the caller. */
  onEditServer?: () => void;
  onInvite?: () => void;
}

/**
 * The server's name and the actions available in its channel rail.
 */
export function RailHeader({
  serverName,
  onCreateChannel,
  onEditServer,
  onInvite,
}: RailHeaderProps) {
  const { t } = useLanguage();
  return (
    <div className="rail-header">
      <h1>{serverName}</h1>
      {onCreateChannel !== undefined && (
        <button
          type="button"
          className="rail-act"
          title={t("channel.create")}
          aria-label={t("channel.create")}
          onClick={onCreateChannel}
        >
          <Icon name="plus" size={16} />
        </button>
      )}
      {onEditServer === undefined ? null : (
        <button
          type="button"
          className="rail-act"
          title={t("shell.editServer")}
          aria-label={t("shell.editServer")}
          onClick={onEditServer}
        >
          <Icon name="edit" size={16} />
        </button>
      )}
      {onInvite === undefined ? null : (
        <button
          type="button"
          className="rail-act invite"
          title={t("shell.inviteToServer")}
          aria-label={t("shell.inviteToServer")}
          onClick={onInvite}
        >
          <Icon name="userAdd" size={17} />
        </button>
      )}
    </div>
  );
}
