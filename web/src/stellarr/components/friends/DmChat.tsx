import { useMemo, type ReactNode } from "react";

import { Avatar } from "@/components/primitives/Avatar";
import type { MessageTranslationOptions } from "@/components/room/attachments/TranslatedTextAttachment";
import { Composer } from "@/components/room/composer/Composer";
import type { ComposerChip } from "@/components/room/composer/ComposerChips";
import { Transcript, type TranscriptSectionVM } from "@/components/room/transcript/Transcript";
import type { OpenProfile, PersonVM } from "@/components/view-models";
import { translateForDocument } from "@/i18n/defaultLocale";
import type { AttachmentCallbacks } from "@/lib/messages/parts";
import type { CustomEmoji } from "@/ports";

export interface DmChatProps {
  peer: PersonVM;
  /** Without the "@"; the intro prints it under the name. */
  username: string;
  sections: TranscriptSectionVM[];
  onSend: (text: string) => Promise<void>;
  hasOlderMessages?: boolean;
  loadingOlderMessages?: boolean;
  onLoadOlderMessages?: () => void;
  chips: ComposerChip[];
  onAttach: (files: File[]) => void;
  onRemoveChip: (chipId: string) => void;
  onRetryChip: (chipId: string) => void;
  onReorderChips: (from: number, to: number) => void;
  /** From an author in the transcript. */
  onOpenProfile: OpenProfile;
  /** The server's own emoji: the picker and the reaction bar both offer them. */
  customEmoji?: readonly CustomEmoji[];
  translation?: MessageTranslationOptions;
  /** The same picture, clip and recording actions the channel transcript uses. */
  attachments?: AttachmentCallbacks;
  /** One action under the intro's sentence. */
  introAction?: ReactNode;
}

/**
 * The messages of one conversation and the field to answer in: the same
 * transcript and composer a channel has, with a history that has a beginning,
 * stated once above the first message rather than assumed.
 *
 * The room shows it, and during the conversation's call so does the column
 * beside the call. It is one piece so the two can never differ.
 */
export function DmChat({
  peer,
  username,
  sections,
  onSend,
  hasOlderMessages = false,
  loadingOlderMessages = false,
  onLoadOlderMessages,
  chips,
  onAttach,
  onRemoveChip,
  onRetryChip,
  onReorderChips,
  onOpenProfile,
  customEmoji,
  translation,
  attachments,
  introAction,
}: DmChatProps) {
  const t = translation?.t ?? translateForDocument;
  const name = peer.displayName;

  const intro = useMemo(
    () => (
      <div className="dm-intro">
        <Avatar
          id={peer.id}
          displayName={name}
          initials={peer.initials}
          avatarUrl={peer.avatarUrl}
          size="xl"
        />
        <h3>{name}</h3>
        <span className="dm-handle">@{username}</span>
        <p>{t("dm.intro")}</p>
        {introAction === undefined ? null : <div className="dm-intro-act">{introAction}</div>}
      </div>
    ),
    [peer.id, peer.initials, peer.avatarUrl, name, username, introAction, t],
  );

  return (
    <>
      <Transcript
        sections={sections}
        label={t("dm.messagesWith", { name })}
        onOpenAuthor={onOpenProfile}
        customEmoji={customEmoji}
        translation={translation}
        attachments={attachments}
        intro={intro}
        hasOlder={hasOlderMessages}
        loadingOlder={loadingOlderMessages}
        onLoadOlder={onLoadOlderMessages}
      />
      <Composer
        channelName={name}
        placeholder={t("dm.writeTo", { name })}
        chips={chips}
        onSubmit={onSend}
        onAttach={onAttach}
        onRemoveChip={onRemoveChip}
        onRetryChip={onRetryChip}
        onReorderChips={onReorderChips}
        customEmoji={customEmoji}
        locale={translation?.locale}
        t={t}
      />
    </>
  );
}
