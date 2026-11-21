import { Icon } from "@stellarr/icons";
import type { Translator } from "@/i18n";

export interface ComposerReplyBarProps {
  authorName: string;
  preview: string;
  onCancel?(): void;
  t: Translator;
}

/** Names whose message is being answered. Sending consumes it; the host clears it. */
export function ComposerReplyBar({ authorName, preview, onCancel, t }: ComposerReplyBarProps) {
  return (
    <div className="comp-reply">
      <span className="comp-reply-txt">
        {t("composer.replyingTo")} <b>{authorName}</b>
        {preview.length === 0 ? null : <i>{preview}</i>}
      </span>
      <button
        type="button"
        className="comp-reply-x"
        aria-label={t("composer.cancelReply")}
        title={t("composer.cancelReply")}
        onClick={onCancel}
      >
        <Icon name="x" size={14} />
      </button>
    </div>
  );
}
