import { deleteMessageConfirmation } from "@/components/confirmations";
import type { IconName } from "@stellarr/icons";
import type { MenuEntry } from "@/components/primitives/ContextMenu";
import type { Confirm } from "@/components/primitives/ConfirmDialog";
import type { MessageKey, Translator } from "@/i18n";

import { absoluteUrl, canCopyImages, copyImage } from "../attachments/mediaActions";
import {
  downloadTarget,
  plainText,
  startDownload,
  LAZY_LINK_PREFIX,
  type DownloadTarget,
  type PointedAt,
} from "@/lib/messages/messageExport";
import type { MessageAction } from "./MessageToolbar";
import type { MessagePart } from "@/lib/messages/parts";

export interface MessageMenuInput {
  messageId: string;
  parts: readonly MessagePart[];
  pinned: boolean;
  /** Where the pointer was, so the menu can offer the picture under it. */
  pointed: PointedAt;
  own: boolean;
  canModerate: boolean;
  /** Adds "copy identifier", as the preference in the advanced settings promises. */
  developerMode: boolean;
  /** Fetches an encrypted original that was left unread until it is saved or copied. */
  resolveOriginal?(src: string): Promise<string | null>;
  /** Absent leaves the menu with only what needs no handler: copying and saving. */
  onAction?(action: MessageAction): void;
  confirm: Confirm;
  t: Translator;
}

/** The clipboard refuses outside a secure context or without permission; nothing to tell the reader. */
function copy(text: string): void {
  void navigator.clipboard?.writeText(text).catch(() => undefined);
}

/** Runs the action on the address, once an original that was left unread has been fetched. */
function withOriginal(
  href: string,
  resolve: MessageMenuInput["resolveOriginal"],
  act: (href: string) => void,
): void {
  if (resolve === undefined || !href.startsWith(LAZY_LINK_PREFIX)) {
    act(href);
    return;
  }
  void resolve(href).then((url) => {
    if (url !== null) act(url);
  });
}

function imageEntries(
  target: DownloadTarget,
  t: Translator,
  resolve: MessageMenuInput["resolveOriginal"],
): MenuEntry[] {
  const entries: MenuEntry[] = [];
  // The desktop client has no browser menu behind this one to fall back on, so
  // "copy image" exists here or nowhere.
  if (canCopyImages()) {
    entries.push({
      id: "copy-image",
      label: t("media.copyImage"),
      icon: "image",
      onSelect: () => {
        // A picture that cannot be read was never going to reach the clipboard;
        // its address, offered next, still can.
        withOriginal(target.href, resolve, (href) => void copyImage(href).catch(() => undefined));
      },
    });
  }
  entries.push({
    id: "copy-image-link",
    label: t("media.copyImageLink"),
    icon: "link",
    onSelect: () => copy(absoluteUrl(target.href)),
  });
  return entries;
}

/**
 * The right-click menu of one message.
 *
 * The same actions the hover toolbar offers, under the same rules, plus the
 * ones that only make sense with a pointer already on the message: copying it,
 * saving what is under the pointer, and copying its id in developer mode.
 * Nothing here is unique to the menu — a menu that is the only way to reach
 * something is a menu that hides it.
 */
export function messageMenuEntries(input: MessageMenuInput): MenuEntry[] {
  const { messageId, parts, pinned, own, canModerate, onAction, confirm, t } = input;
  const entries: MenuEntry[] = [];

  if (onAction !== undefined) {
    const action = (id: MessageAction, label: MessageKey, icon: IconName): MenuEntry => ({
      id,
      label: t(label),
      icon,
      onSelect: () => onAction(id),
    });
    entries.push(action("reply", "message.action.reply", "reply"));
    entries.push(action("thread", "message.action.thread", "thread"));
    if (own) entries.push(action("edit", "message.action.edit", "edit"));
    // The label is read off the message so the menu and the toolbar cannot disagree.
    if (canModerate) {
      entries.push(
        action(
          "pin",
          pinned ? "message.action.unpinMessage" : "message.action.pinMessage",
          "pin",
        ),
      );
    }
  }

  entries.push("separator");
  const file = downloadTarget(parts, input.pointed);
  if (file !== undefined) {
    entries.push({
      id: "download",
      label: t("common.download"),
      icon: "download",
      onSelect: () => withOriginal(file.href, input.resolveOriginal, (href) => startDownload({ ...file, href })),
    });
    if (file.image === true) entries.push(...imageEntries(file, t, input.resolveOriginal));
  }
  entries.push({
    id: "copy",
    label: t("message.action.copyText"),
    icon: "copy",
    onSelect: () => copy(plainText(parts)),
  });
  if (input.developerMode) {
    entries.push({
      id: "copy-id",
      label: t("message.action.copyId"),
      icon: "clipboard",
      onSelect: () => copy(messageId),
    });
  }

  if ((own || canModerate) && onAction !== undefined) {
    entries.push("separator", {
      id: "delete",
      label: t("message.action.deleteMessage"),
      icon: "trash",
      danger: true,
      // The menu closes on select, so the question cannot belong to it —
      // `confirm` raises the app's dialog from the root and answers here.
      onSelect: () => {
        void confirm(deleteMessageConfirmation(t)).then((confirmed) => {
          if (confirmed) onAction("delete");
        });
      },
    });
  }
  return entries;
}
