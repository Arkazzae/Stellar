import { useLayoutEffect, useRef, useState, type RefObject } from "react";

import { deleteMessageConfirmation } from "@/components/confirmations";
import { Icon, type IconName } from "@stellarr/icons";
import { useConfirm, type ConfirmSubject } from "@/components/primitives/ConfirmDialog";
import { type MessageKey, type Translator } from "@/i18n";
import { translateForDocument } from "@/i18n/defaultLocale";
import type { CustomEmoji } from "@/ports";

import { MessageActionSheet, type SheetItem } from "./MessageActionSheet";
import {
  placeReactionPopover,
  ReactionPopover,
  type PopoverPlacement,
} from "./ReactionPopover";
import { onPhone, presenceIn, type ToolbarPresence } from "./toolbarPresence";

export type MessageAction = "reply" | "thread" | "edit" | "pin" | "delete";

interface Viewer {
  /** This message is yours. */
  own: boolean;
  /** You may moderate this channel. */
  canModerate: boolean;
}

interface ActionSpec {
  action: MessageAction;
  icon: IconName;
  label: MessageKey;
  /** Who is offered it. Absent means everyone; a refused action is left out, not greyed. */
  allowedFor?(viewer: Viewer): boolean;
  /** Leaves the compact desktop toolbar first; the phone sheet keeps it. */
  secondary?: boolean;
  danger?: boolean;
}

const ACTIONS: readonly ActionSpec[] = [
  { action: "reply", icon: "reply", label: "message.action.reply" },
  {
    action: "thread",
    icon: "thread",
    label: "message.action.thread",
    secondary: true,
  },
  {
    action: "edit",
    icon: "edit",
    label: "message.action.edit",
    // Only your own. Rewriting somebody else's message leaves their name over
    // words they never wrote, which no amount of moderator is a reason for.
    allowedFor: ({ own }) => own,
  },
  {
    action: "pin",
    icon: "pin",
    label: "message.action.pin",
    // Anyone's message, and only a moderator's. A pin puts one message above
    // the conversation for everybody who opens the channel, so it is not a
    // bookmark you keep for yourself; the server says the same.
    allowedFor: ({ canModerate }) => canModerate,
    // Deliberate and rare, so the first to fold away from a narrow toolbar.
    secondary: true,
  },
  {
    action: "delete",
    icon: "trash",
    label: "message.action.delete",
    allowedFor: ({ own, canModerate }) => own || canModerate,
    danger: true,
  },
];

export interface MessageToolbarProps {
  /** Held by the message these actions belong to; see `MessageEntry`. */
  presence: RefObject<ToolbarPresence | null>;
  onReact?(emoji: string): void;
  /** A deletion's promise keeps the question card open until it settles. */
  onAction?(action: MessageAction): Promise<void> | void;
  /** The message as the question card shows it, built when the question is asked. */
  describe?(): ConfirmSubject;
  /** The server's own emoji, offered first in the picker. */
  customEmoji?: readonly CustomEmoji[];
  /** This message is yours: yours to edit, and yours to delete. */
  own?: boolean;
  /** You may moderate this channel, which is a licence to delete, not to edit. */
  canModerate?: boolean;
  /** Already pinned: the same control unpins it, and says so. */
  pinned?: boolean;
  t?: Translator;
}

interface OpenPicker {
  placement: PopoverPlacement;
  autoFocus: boolean;
}

interface ToolbarButtonsProps {
  actions: readonly ActionSpec[];
  labelOf(spec: ActionSpec): string;
  pinned: boolean;
  pickerOpen: boolean;
  chooseReactionLabel: string;
  onPicker(): void;
  onRun(spec: ActionSpec): void;
}

/**
 * What the hover toolbar holds. It is drawn apart from the transcript, so it
 * reads no context: everything it shows comes in here.
 */
function ToolbarButtons({
  actions,
  labelOf,
  pinned,
  pickerOpen,
  chooseReactionLabel,
  onPicker,
  onRun,
}: ToolbarButtonsProps) {
  return (
    <>
      <button
        type="button"
        className={pickerOpen ? "tb emo-more on" : "tb emo-more"}
        aria-label={chooseReactionLabel}
        aria-haspopup="dialog"
        aria-expanded={pickerOpen}
        onClick={onPicker}
      >
        <Icon name="smiley" size={15} />
      </button>
      <span className="tb-sep" aria-hidden="true" />
      {actions.map((spec) => {
        const label = labelOf(spec);
        const isPin = spec.action === "pin";
        return (
          <button
            key={spec.action}
            type="button"
            className={[
              "tb",
              spec.secondary === true && "tb-x",
              spec.danger === true && "danger",
              isPin && pinned && "on",
            ]
              .filter(Boolean)
              .join(" ")}
            aria-label={label}
            title={label}
            aria-pressed={isPin ? pinned : undefined}
            onClick={() => onRun(spec)}
          >
            {/* The same glyph pinned or not: `aria-pressed` and the tint say
                which, the way the composer's code toggle does. */}
            <Icon name={spec.icon} size={isPin ? 17 : 15} />
          </button>
        );
      })}
    </>
  );
}

/** The picker's own button, in whichever toolbar hangs in the message's corner. */
function pickerButtonOf(
  presence: RefObject<ToolbarPresence | null>,
): RefObject<HTMLButtonElement | null> {
  return {
    get current() {
      return presence.current?.toolbar?.querySelector<HTMLButtonElement>(".emo-more") ?? null;
    },
  };
}

/**
 * A message's actions: the corner its hover toolbar hangs in, and the one
 * control that stands for the toolbar there.
 *
 * The control is always there. On a phone it is the visible disclosure that
 * opens the sheet; on a wide screen nobody sees it, and focus that reaches it
 * passes on to the toolbar's buttons, which then take its place in the tab
 * order, so Tab and Shift+Tab meet the same buttons in the same order as when
 * every toolbar was in the document.
 */
export function MessageToolbar({
  presence,
  onReact,
  onAction,
  describe,
  customEmoji = [],
  own = false,
  canModerate = false,
  pinned = false,
  t = translateForDocument,
}: MessageToolbarProps) {
  const confirm = useConfirm();
  const [picker, setPicker] = useState<OpenPicker | null>(null);
  const [sheetOpen, setSheetOpen] = useState(false);
  const corner = useRef<HTMLDivElement>(null);
  const standIn = useRef<HTMLButtonElement>(null);
  const [pickerTrigger] = useState(() => pickerButtonOf(presence));
  // Whether the last press was a finger or a pen. The picker then opens without
  // focusing its search field, which would raise the on-screen keyboard over it.
  const touched = useRef(false);
  // Pressed rather than tabbed to, which has no direction.
  const pressing = useRef(false);

  const viewer: Viewer = { own, canModerate };
  const actions = ACTIONS.filter((spec) => spec.allowedFor?.(viewer) ?? true);

  // One button in both directions: the label follows the message rather than a
  // second control appearing beside the first.
  const labelOf = ({ action, label }: ActionSpec) =>
    action === "pin" && pinned ? t("message.action.unpin") : t(label);

  const run = ({ action }: ActionSpec) => {
    if (onAction === undefined) return;
    if (action !== "delete") {
      void onAction(action);
      return;
    }
    void confirm({
      ...deleteMessageConfirmation(t, describe?.()),
      run: async () => onAction(action),
    });
  };

  const openPicker = () => {
    // On a phone the toolbar is not shown; the control that opened the sheet
    // stands where it would be.
    const anchor = (onPhone() ? standIn.current : (presence.current?.toolbar ?? standIn.current))
      ?.getBoundingClientRect();
    if (anchor === undefined) return;
    setPicker({ placement: placeReactionPopover(anchor), autoFocus: !touched.current });
  };

  const closePicker = (restoreFocus: boolean) => {
    setPicker(null);
    if (!restoreFocus) return;
    const trigger = onPhone() ? standIn : pickerTrigger;
    trigger.current?.focus();
  };

  const focusToolbar = (which: "first" | "last") => {
    const toolbar = presenceIn(presence);
    toolbar.focused = true;
    toolbar.update();
    const buttons = toolbar.toolbar?.querySelectorAll<HTMLButtonElement>("button");
    if (buttons === undefined || buttons.length === 0) return;
    // The stand-in has scrolled the transcript as far as focus on this
    // button scrolled it while the toolbar was always there.
    buttons[which === "first" ? 0 : buttons.length - 1]?.focus({ preventScroll: true });
  };

  const sheetItems: SheetItem[] = [
    {
      id: "react",
      label: t("message.action.chooseReaction"),
      icon: "smiley",
      onSelect: openPicker,
    },
    ...actions.map((spec) => ({
      id: spec.action,
      label: labelOf(spec),
      icon: spec.icon,
      danger: spec.danger,
      pressed: spec.action === "pin" ? pinned : undefined,
      onSelect: () => run(spec),
    })),
  ];

  // What the toolbar shows when it hangs, kept current with every drawing of
  // the message.
  const content = (
    <ToolbarButtons
      actions={actions}
      labelOf={labelOf}
      pinned={pinned}
      pickerOpen={picker !== null}
      chooseReactionLabel={t("message.action.chooseReaction")}
      onPicker={() => (picker === null ? openPicker() : closePicker(false))}
      onRun={run}
    />
  );
  const label = t("message.actions");
  const engaged = picker !== null || sheetOpen;
  useLayoutEffect(() => {
    const toolbar = presenceIn(presence);
    toolbar.corner = corner.current;
    toolbar.standIn = standIn.current;
    toolbar.label = label;
    toolbar.content = content;
    toolbar.engaged = engaged;
    toolbar.settle();
  });
  useLayoutEffect(() => {
    const toolbar = presenceIn(presence);
    return () => toolbar.dispose();
  }, [presence]);

  return (
    <div
      ref={corner}
      className="msg-actions"
      // Captured here because the sheet is portaled and the toolbar is drawn
      // apart, and their presses still reach this element.
      onPointerDownCapture={(event) => {
        touched.current = event.pointerType !== "mouse";
      }}
      onKeyDownCapture={() => {
        touched.current = false;
      }}
    >
      <button
        ref={standIn}
        type="button"
        className="tb msg-mobile-more"
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={sheetOpen}
        onFocus={(event) => {
          if (onPhone()) return;
          // Shift+Tab arrives from further down the page and lands on the
          // toolbar's last button, as it did when the toolbar was always there.
          const from = event.relatedTarget;
          const fromBelow =
            !pressing.current &&
            from instanceof Node &&
            (event.currentTarget.compareDocumentPosition(from) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
          // As focus would, while the stand-in is still there to scroll to.
          event.currentTarget.scrollIntoView({ block: "nearest", inline: "nearest" });
          focusToolbar(fromBelow ? "last" : "first");
        }}
        onClick={(event) => {
          if (onPhone()) {
            setSheetOpen((open) => !open);
            return;
          }
          // Pressed without being focused, as a screen reader may: the same
          // as arriving by Tab from above.
          pressing.current = true;
          event.currentTarget.focus();
          pressing.current = false;
        }}
      >
        <Icon name="dots" size={18} />
      </button>
      {picker === null ? null : (
        <ReactionPopover
          placement={picker.placement}
          label={t("message.action.chooseReaction")}
          autoFocus={picker.autoFocus}
          customEmoji={customEmoji}
          triggers={[pickerTrigger, standIn]}
          // A reaction is one emoji, so the panel closes behind it — unlike
          // the composer's, where three in a row is the normal case.
          onPick={(reaction) => {
            onReact?.(reaction);
            closePicker(false);
          }}
          onClose={closePicker}
        />
      )}
      {sheetOpen ? (
        <MessageActionSheet
          title={label}
          closeLabel={t("common.close")}
          items={sheetItems}
          returnFocusTo={standIn}
          onClose={() => setSheetOpen(false)}
        />
      ) : null}
    </div>
  );
}
