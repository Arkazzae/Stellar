import {
  useEffect,
  useId,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
} from "react";

import { Icon } from "@stellarr/icons";
import { ContextMenu } from "@/components/primitives/ContextMenu";
import { type InterfaceLocale, type MessageKey, type Translator } from "@/i18n";
import { documentLocale, translateForDocument } from "@/i18n/defaultLocale";
import type { CustomEmoji, OpenVoiceMessageCapture } from "@/ports";

import { AUTO_LANGUAGE } from "@/components/room/code/CodeLanguagePicker";
import { ComposerChips, type ComposerChip } from "./ComposerChips";
import { ComposerReplyBar } from "./ComposerBars";
import { ComposerCodeHead, ComposerCodeLayers, composerCodeStyles as codeCss } from "./ComposerCode";
import { ComposerFormatPanel } from "./ComposerFormatPanel";
import { CommandArgs } from "./CommandArgs";
import { CommandList, commandOptionId } from "./CommandList";
import { CommandSheet, type SheetEntry, type SheetGroup } from "./CommandSheet";
import {
  COMMANDS,
  argumentProblems,
  argumentsComplete,
  type CommandContext,
  type CommandInput,
  type ComposerCommand,
} from "./commandRegistry";
import commandCss from "./ComposerCommands.module.css";
import { ScheduledBar, SchedulePicker, useClock, useNarrowForm, WhenChip } from "./ComposerSchedule";
import scheduleCss from "./ComposerSchedule.module.css";
import { presetRow, whenLabel } from "@/lib/schedule/scheduleLabels";
import { parseWhen, schedulePresets, timeProblem, type TimeProblem } from "@/lib/schedule/scheduleTime";
import { codeLanguageNamed } from "@/lib/messages/codeLanguages";
import { namesCommand } from "@/lib/messages/composerCommands";
import { insertMention, type MentionCandidate } from "@/lib/messages/composerMentions";
import { insertEmojiAt, insertSpoiler, toggleMarker } from "@/lib/messages/composerText";
import { EmojiPicker } from "@/components/emoji/EmojiPicker";
import { GifBrowser, type GifBrowserHandle } from "./GifBrowser";
import { MentionList, mentionOptionId } from "./MentionList";
import { CODE_MESSAGE_MAX_LENGTH } from "@/lib/messages/messageLimits";
import { useAutosizedField } from "./useAutosizedField";
import {
  commandInput,
  useComposerCommands,
  type CommandBase,
  type CommandItem,
} from "./useComposerCommands";
import { useComposerDraft } from "./useComposerDraft";
import { useComposerPopovers } from "./useComposerPopovers";
import { useDismissible } from "./useDismissible";
import { useMentionSuggestions } from "./useMentionSuggestions";
import { VoiceRecorder } from "./VoiceRecorder";
import { indentCode, type RangeEdit } from "./codeEditing";

export type { MentionCandidate };

/** What Tab would stop at inside the composer. */
const FOCUSABLE =
  'button:not(:disabled), input:not(:disabled):not([type="hidden"]), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/**
 * Moves focus to the control before or after `field` in the composer, as Tab
 * and Shift+Tab would. Controls that are not drawn, such as the phone's
 * disclosure on a desktop, are passed over where the browser can tell.
 */
function focusBeside(field: HTMLTextAreaElement, direction: 1 | -1) {
  const form = field.form;
  if (form === null) return;
  const controls = Array.from(form.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (control) =>
      control === field ||
      (!control.hidden && (control.checkVisibility?.() ?? true)),
  );
  controls[controls.indexOf(field) + direction]?.focus();
}

function countLines(text: string): number {
  let lines = 1;
  for (let at = text.indexOf("\n"); at !== -1; at = text.indexOf("\n", at + 1)) lines += 1;
  return lines;
}

const MAX_ATTACHMENTS = 10;
/** How tall the code editor grows before it scrolls: about twelve lines at desktop size. */
const CODE_FIELD_MAX_HEIGHT = 250;

/**
 * What the file picker accepts: everything the composer can queue. The
 * narrower lists on the document and picture buttons are shortcuts to one
 * kind, not the only way in.
 */
const ANY_ATTACHMENT = [
  "image/*",
  "video/*",
  "audio/*",
  "application/pdf",
  "application/zip",
  "application/json",
  "text/plain",
  ".docx",
  ".xlsx",
  ".pptx",
].join(",");

const NO_INPUT: CommandInput = commandInput({ text: {}, people: {} });
/** How long a finger holds the send button before it offers sending later. */
const LONG_PRESS_MS = 500;

const WHEN_PROBLEMS: Record<TimeProblem | "unknown", MessageKey> = {
  unknown: "composer.schedule.parse.unknown",
  past: "composer.schedule.passed",
  tooSoon: "composer.schedule.passed",
  tooFar: "composer.schedule.tooFar",
};
const NO_PEOPLE: readonly MentionCandidate[] = [];

/**
 * What the commands reach outside the composer. Each one left out hides its
 * command, so a host offers only what exists where it is and what the person
 * may do there.
 */
export interface ComposerCommandHost {
  openPinned?(): void;
  openStatus?(): void;
  /** Asks on the question card; the command never kicks by itself. */
  kick?(person: MentionCandidate): void;
  /** Asks on the question card, with the reason in its field. */
  ban?(person: MentionCandidate, reason: string): void;
  /** Who a person argument offers; the host leaves out whoever cannot be chosen. */
  people?: readonly MentionCandidate[];
}

/** Sending later, where the message can be scheduled. */
export interface ComposerScheduling {
  /** Schedules instead of posting; the text is trimmed and fenced as a send's would be. */
  onSchedule(text: string, sendAt: Date): Promise<void>;
  /** What the author has waiting here, for the bar over the field. */
  waiting: { count: number; next: Date | null };
  /** Which words the bar uses for this place. */
  place: "channel" | "thread" | "conversation";
  onShowScheduled(): void;
  /** The reader's time zone, in which times are chosen and shown. */
  zone: string;
  /** The real clock unless a test or a picture fixes it. */
  clock?: () => Date;
}

export interface ComposerProps {
  draftScope?: string;
  restoringFiles?: boolean;
  /** Bare channel name; the placeholder prefixes it with "#". */
  channelName: string;
  /**
   * Replaces the channel phrasing outright. A conversation is written to a
   * person, not on a channel, so a prefix would not have been enough.
   */
  placeholder?: string;
  /** Changing this scope cancels an open microphone capture. */
  recordingScope?: string | null;
  /**
   * False disables the attachment controls that stay on screen (code mode's
   * plus, the microphone) and says why on hover: a place with nowhere to
   * upload to should not queue a file nothing will ever send. The /file
   * command, which is only a way in, is not offered at all.
   */
  attachmentsSupported?: boolean;
  chips: ComposerChip[];
  /**
   * Whose message this one answers, if any. The bar above the field names
   * them; sending is what consumes it, and the host clears it.
   */
  replyingTo?: { id: string; authorName: string; preview: string } | null;
  onCancelReply?(): void;
  /** Text is already trimmed. Chips are the caller's to consume and clear. */
  onSubmit(text: string): Promise<void> | void;
  onRemoveChip(id: string): void;
  onRetryChip(id: string): void;
  onReorderChips(from: number, to: number): void;
  /** Raised on keystrokes so a container can throttle a typing notification. */
  onTyping?(): void;
  onAttach?(files: File[]): void;
  /** Sends a GIF; without it the composer offers no GIF button. */
  onGif?(pageUrl: string): Promise<void>;
  /** The server's own emoji, offered first in the picker. */
  customEmoji?: readonly CustomEmoji[];
  /** Server members offered after typing @. */
  mentionCandidates?: readonly MentionCandidate[];
  openVoiceMessageMicrophone?: OpenVoiceMessageCapture;
  onSendVoice?(file: File, durationSeconds: number): Promise<void>;
  /** The commands that lead out of the composer. */
  commandHost?: ComposerCommandHost;
  /** Without it the message can only be sent now. */
  scheduling?: ComposerScheduling;
  /** Opens a new poll with the question typed; without it no poll is offered. */
  onPoll?(question: string): void;
  /** Opens a new event; without it, as for whoever may not create one, none is offered. */
  onEvent?(): void;
  locale?: InterfaceLocale;
  t?: Translator;
}

export function Composer({
  channelName,
  placeholder,
  recordingScope = channelName,
  draftScope,
  restoringFiles = false,
  attachmentsSupported = true,
  chips,
  replyingTo = null,
  onCancelReply,
  onSubmit,
  onRemoveChip,
  onRetryChip,
  onReorderChips,
  onTyping,
  onAttach,
  onGif,
  customEmoji = [],
  mentionCandidates = [],
  openVoiceMessageMicrophone,
  onSendVoice,
  commandHost,
  scheduling,
  onPoll,
  onEvent,
  locale = documentLocale(),
  t = translateForDocument,
}: ComposerProps) {
  const [voiceOpen, setVoiceOpen] = useState(false);
  const [observedScope, setObservedScope] = useState(recordingScope);
  /**
   * The phone's sheet, while it is open. Its entries are made when it opens,
   * for the draft it opened on: nothing can change under a modal sheet.
   */
  const [sheet, setSheet] = useState<{ tiles: SheetEntry[]; groups: SheetGroup[] } | null>(null);
  const sheetOpen = sheet !== null;
  /** The send-later menu, where it opens, while it is open. */
  const [sendMenu, setSendMenu] = useState<{ x: number; y: number } | null>(null);
  /** The date picker, while it is open, with the time it opens on. */
  const [picker, setPicker] = useState<{ initial: Date | null } | null>(null);
  const press = useRef<number | null>(null);
  /** A long press opened the menu: the click that ends it must not also send. */
  const swallowClick = useRef(false);

  const field = useRef<HTMLTextAreaElement | null>(null);
  const filePicker = useRef<HTMLInputElement | null>(null);
  const commandTrigger = useRef<HTMLButtonElement | null>(null);
  const commandPop = useRef<HTMLDivElement | null>(null);
  const gifBrowser = useRef<GifBrowserHandle | null>(null);
  const mobileActionsId = useId();
  const mentionListId = useId();
  const commandListId = useId();
  const commandHintId = useId();
  const codeHelpId = useId();
  const codeCounterId = useId();
  /**
   * Set by Escape in code mode, where Tab indents: the next Tab or Shift+Tab
   * then moves to the control after or before the field instead, so the
   * keyboard is never trapped in it.
   */
  const releaseTab = useRef(false);
  /** An input method is composing a word in the field. */
  const [composing, setComposing] = useState(false);
  /**
   * Where the caret is, for the picker.
   *
   * Clicking into the panel takes focus off the textarea, and a touch-opened
   * picker never gives it back, so the field's own selection stops following
   * the inserts. It is kept up to date while the field has focus and advanced
   * by hand after each insert.
   */
  const caret = useRef<number | null>(null);
  /** The same caret as state, for the command list, which follows it. */
  const [caretAt, setCaretAt] = useState(0);

  const {
    emojiOpen,
    setEmojiOpen,
    emojiAutoFocus,
    setEmojiAutoFocus,
    formatOpen,
    setFormatOpen,
    mobileActionsOpen,
    setMobileActionsOpen,
    emojiTrigger,
    emojiPanel,
    formatTrigger,
    formatPanel,
    mobileActionsTrigger,
    mobileActions,
  } = useComposerPopovers();
  const {
    text,
    setText,
    sending,
    sendError,
    clearSendError,
    loading: loadingDraft,
    codeMode,
    automaticCode,
    setCodeMode,
    codeLanguage,
    setCodeLanguage,
    codeGuess,
    codeHighlight,
    codeRoom,
    sendAt,
    setSendAt,
    hasPayload,
    attachmentsBlocked,
    canSend,
    submit,
  } = useComposerDraft({
    draftScope,
    restoringFiles,
    chips,
    onSubmit,
    ...(scheduling === undefined
      ? {}
      : { onSchedule: scheduling.onSchedule, ...(scheduling.clock === undefined ? {} : { clock: scheduling.clock }) }),
    onSent: () => setMobileActionsOpen(false),
    locale,
    t,
  });

  const voiceUnavailable =
    onSendVoice === undefined || openVoiceMessageMicrophone === undefined;
  const attachmentsFull = chips.length >= MAX_ATTACHMENTS;

  // A recording belongs to the room in which it started. Adjusted during
  // render so the old room's microphone never survives into the new room for
  // a committed frame; unmounting VoiceRecorder releases the capture.
  if (observedScope !== recordingScope) {
    setObservedScope(recordingScope);
    setVoiceOpen(false);
    setMobileActionsOpen(false);
  }

  // Starting a reply puts the cursor where the answer goes. Keyed on the
  // answered message, not on the prop object, which the host rebuilds on every
  // render: depending on that would steal the caret on every keystroke.
  const answering = replyingTo?.id ?? null;
  useEffect(() => {
    if (answering !== null) field.current?.focus();
  }, [answering]);

  /** Focuses the field and selects once the new text has been rendered into it. */
  const selectLater = (from: number, to = from) => {
    requestAnimationFrame(() => {
      const target = field.current;
      if (target === null) return;
      target.focus();
      target.setSelectionRange(from, to);
    });
  };

  const openPicker = (accept: string) => {
    const picker = filePicker.current;
    if (picker === null || onAttach === undefined) return;
    picker.accept = accept;
    picker.click();
  };

  const switchCodeMode = (enabled: boolean) => {
    setCodeMode(enabled);
    // Neither panel has anything to offer a code block.
    setFormatOpen(false);
    if (enabled) setEmojiOpen(false);
    setMobileActionsOpen(false);
    releaseTab.current = false;
  };

  /**
   * The question card gives the focus back to whatever had it when it opened.
   * A command's fields leave as it runs, so the field takes the focus once it
   * is back on the page, and only then is the card asked for.
   */
  const askAfterFrame = (ask: () => void) => {
    requestAnimationFrame(() => {
      field.current?.focus();
      ask();
    });
  };

  const now = useClock(scheduling?.clock);
  const formElement = useRef<HTMLFormElement>(null);
  /** Where the row has no room for the time beside the words, it waits over the box. */
  const timeOverBox = useNarrowForm(formElement) || codeMode;
  const zone = scheduling?.zone ?? "UTC";
  /** Nobody is told someone is writing a message that will appear only later. */
  const typing = () => {
    if (sendAt === null) onTyping?.();
  };
  /** A typed send time, read against the clock on screen. */
  const readWhen = (text: string): { at: Date } | { problem: MessageKey } => {
    const parsed = parseWhen(text, now, zone, locale);
    return "at" in parsed ? parsed : { problem: WHEN_PROBLEMS[parsed.problem] };
  };
  const chooseTime = (at: Date, caretAt: number) => {
    setSendAt(at);
    selectLater(caretAt);
  };

  const host = commandHost ?? {};
  const { openPinned, openStatus, kick, ban } = host;

  /** What the commands can do, for the draft a command would leave behind. */
  const contextFor = (base: CommandBase): CommandContext => ({
    openFilePicker:
      attachmentsSupported && onAttach !== undefined ? () => openPicker(ANY_ATTACHMENT) : undefined,
    fileBlocked: attachmentsFull ? "composer.attachmentLimit" : undefined,
    openGifs: onGif === undefined ? undefined : (query) => gifBrowser.current?.open(query),
    openPoll: onPoll,
    openEvent: onEvent,
    recordVoice: attachmentsSupported && !voiceUnavailable ? () => setVoiceOpen(true) : undefined,
    // Judged on the draft the command leaves: the "/voice" being typed is not
    // a message that would have to go first.
    voiceBlocked: sending
      ? "composer.sending"
      : chips.length > 0 || base.text.trim() !== ""
        ? "composer.voiceMustBeSeparate"
        : undefined,
    enterCode: (language) => {
      // A name it does not know is left to the automatic guess rather than
      // turned into plain text.
      setCodeLanguage(codeLanguageNamed(language) ?? AUTO_LANGUAGE);
      switchCodeMode(true);
      selectLater(base.caret);
    },
    insertSpoiler: (words) => {
      const edit = insertSpoiler(base.text, base.caret, words);
      setText(edit.text);
      trackCaret(edit.selectionStart);
      selectLater(edit.selectionStart, edit.selectionEnd);
    },
    openPinned:
      openPinned === undefined
        ? undefined
        : () => {
            openPinned();
            selectLater(base.caret);
          },
    openStatus,
    kick: kick === undefined ? undefined : (person) => askAfterFrame(() => kick(person)),
    ban: ban === undefined ? undefined : (person, reason) => askAfterFrame(() => ban(person, reason)),
    schedule:
      scheduling === undefined
        ? undefined
        : {
            parse: readWhen,
            choose: (at) => chooseTime(at, base.caret),
            openPicker: () => setPicker({ initial: null }),
          },
  });

  const commands = useComposerCommands({
    text,
    caret: caretAt,
    enabled: !codeMode,
    contextFor,
    t,
  });
  const active = commands.active;
  useAutosizedField(
    field,
    text,
    // A chosen time widens the send button, and its chip may share the row:
    // both narrow the field without changing its text.
    `${recordingScope ?? ""}|${sendAt === null ? "send" : timeOverBox ? "over" : "inline"}`,
    codeMode ? CODE_FIELD_MAX_HEIGHT : undefined,
    active !== null,
  );
  const activeInput = active === null ? null : commandInput(active);
  const activeProblems =
    active === null || activeInput === null
      ? {}
      : argumentProblems(active.command, activeInput, {
          ...(scheduling === undefined ? {} : { parseWhen: readWhen }),
        });
  const activeReady =
    active !== null &&
    activeInput !== null &&
    argumentsComplete(active.command, activeInput) &&
    Object.keys(activeProblems).length === 0;

  const mentions = useMentionSuggestions({
    text,
    candidates: mentionCandidates,
    locale,
    enabled: !codeMode && active === null,
  });
  // One list at a time over the field; the commands are what it was opened for.
  const mentionsShown = mentions.open && !commands.open;

  // A list opened from the plus closes on a press beside it, and on Escape
  // from wherever the focus is, handing it back to the field.
  useDismissible(commands.mode === "plus", {
    isInside: (target) =>
      [commandTrigger, commandPop, field].some((part) => part.current?.contains(target) === true),
    onDismiss: () => commands.close(),
    returnFocusTo: field,
  });

  const trackCaret = (position: number) => {
    caret.current = position;
    setCaretAt(position);
    mentions.track(position);
  };

  const chooseMention = (candidate: MentionCandidate) => {
    if (mentions.mention === null) return;
    const edit = insertMention(text, mentions.mention, candidate.username);
    setText(edit.text);
    trackCaret(edit.selectionEnd);
    typing();
    selectLater(edit.selectionEnd);
  };

  /** Leaves the draft as the command found it, then does what the command is for. */
  const runCommand = (command: ComposerCommand, input: CommandInput, base: CommandBase) => {
    setText(base.text);
    trackCaret(base.caret);
    // In the same task as the key or the press: a file picker opened any
    // later is refused for want of a gesture.
    command.run(contextFor(base), input);
  };

  /** Puts a command in the field to be given its arguments. */
  const beginCommand = (command: ComposerCommand, name: string, base: CommandBase, how: "plus" | "slash") => {
    commands.begin(command, name, base, how);
    setText(base.text);
    trackCaret(base.caret);
  };

  /**
   * Enter runs the selected command, or gives it its arguments. Tab does the
   * same for a command with arguments; for one without, it completes the
   * typed name, so Enter is what runs it.
   */
  const chooseCommand = (item: CommandItem, how: "use" | "complete") => {
    const base = commands.base;
    if (item.blocked !== undefined || base === null) return;
    if (item.command.args.length > 0) {
      beginCommand(item.command, item.name, base, commands.mode ?? "plus");
      return;
    }
    if (how === "complete" && commands.slash !== null) {
      const typed = `/${item.name}`;
      setText(`${typed}${text.slice(commands.slash.end)}`);
      trackCaret(typed.length);
      selectLater(typed.length);
      return;
    }
    commands.close();
    runCommand(item.command, NO_INPUT, base);
  };

  const runActive = () => {
    if (active === null || activeInput === null || !activeReady) return;
    commands.end();
    runCommand(active.command, activeInput, active.base);
  };

  /**
   * Out of the command and back to writing. From Escape or Backspace a typed
   * command stays as text, to be edited or sent as it is; the cancel button,
   * or a command chosen from the plus, leaves the draft as it was.
   */
  const leaveCommand = (keepTyped: boolean) => {
    if (active === null) return;
    commands.end();
    if (keepTyped && active.from === "slash") {
      const typed = `/${active.name}`;
      setText(active.base.text === "" ? typed : `${typed} ${active.base.text}`);
      trackCaret(typed.length);
      selectLater(typed.length);
      return;
    }
    setText(active.base.text);
    trackCaret(active.base.caret);
    selectLater(active.base.caret);
  };

  /**
   * A space after a whole command name, typed at the end of the draft, takes
   * the command up as if it had been chosen, so the arguments can be written
   * straight after it, unless Escape put the list away on that word.
   */
  const takeTypedCommand = (value: string, at: number): boolean => {
    if (codeMode || composing || active !== null || at !== value.length) return false;
    const typed = /^\/(\S+) $/.exec(value);
    // Escape on the word said it is text: the space after it is just a space.
    if (typed === null || commands.dismissed === value.slice(0, -1)) return false;
    const word = typed[1] ?? "";
    const base: CommandBase = { text: "", caret: 0 };
    const context = contextFor(base);
    const command = COMMANDS.find((entry) => {
      if (entry.args.length === 0) return false;
      const availability = entry.available(context);
      return (
        availability.shown &&
        availability.blocked === undefined &&
        namesCommand(word, { name: t(entry.name), alias: entry.alias })
      );
    });
    if (command === undefined) return false;
    beginCommand(command, t(command.name), base, "slash");
    return true;
  };

  /** On the phone's row the formatting button is not drawn: it lives in the sheet. */
  const compactRow = () => {
    const format = formatTrigger.current;
    return format !== null && window.getComputedStyle(format).display === "none";
  };

  const onCommandsButton = () => {
    if (sheetOpen) {
      setSheet(null);
      return;
    }
    if (commands.mode === "plus") {
      commands.close();
      field.current?.focus();
      return;
    }
    // Pressed while a command is being given its arguments: back to the
    // draft, and the list to choose another.
    const from: CommandBase =
      active !== null
        ? active.base
        : { text, caret: caret.current ?? field.current?.selectionStart ?? text.length };
    if (active !== null) {
      commands.end();
      setText(from.text);
      trackCaret(from.caret);
    }
    if (compactRow()) {
      setSheet(sheetContent(from));
      return;
    }
    commands.openFromPlus(from);
    setFormatOpen(false);
    // The list is the field's popup and is driven from it.
    selectLater(from.caret);
  };

  const insertEmoji = (glyph: string, restoreFocus: boolean) => {
    const at = caret.current ?? field.current?.selectionStart ?? text.length;
    const edit = insertEmojiAt(text, at, glyph);
    setText(edit.text);
    caret.current = edit.selectionEnd;
    // A touch-opened catalogue is meant for picking several emoji in a row.
    // Focusing the field would raise the software keyboard after the first
    // tap and cover the part of the sheet that is useful.
    if (restoreFocus) selectLater(edit.selectionEnd);
  };

  const applyFormat = (marker: string) => {
    const element = field.current;
    const start = element?.selectionStart ?? caret.current ?? text.length;
    const end = element?.selectionEnd ?? start;
    const edit = toggleMarker(text, start, end, marker);
    setText(edit.text);
    caret.current = edit.selectionEnd;
    setFormatOpen(false);
    selectLater(edit.selectionStart, edit.selectionEnd);
  };

  /**
   * Replaces a range of the draft as if it were typed, so the browser's undo
   * takes it back in one step. Where the browser cannot (a test's document),
   * the draft is set directly.
   */
  const applyCodeEdit = (element: HTMLTextAreaElement, edit: RangeEdit) => {
    element.setSelectionRange(edit.from, edit.to);
    const typed =
      typeof document.execCommand === "function" &&
      (edit.insert.length > 0
        ? document.execCommand("insertText", false, edit.insert)
        : document.execCommand("delete"));
    if (typed) {
      element.setSelectionRange(edit.selectionStart, edit.selectionEnd);
      return;
    }
    setText(`${element.value.slice(0, edit.from)}${edit.insert}${element.value.slice(edit.to)}`);
    typing();
    selectLater(edit.selectionStart, edit.selectionEnd);
  };

  /**
   * Code mode keeps Enter for new lines (Shift+Enter too) and sends on
   * Ctrl+Enter or Cmd+Enter. Tab and Shift+Tab indent and outdent; after
   * Escape, Tab leaves the field instead.
   */
  const onCodeKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // An input method's own keys pick and confirm a word: none of them may
    // send, indent or arm the way out. Safari reports the confirming key only
    // by its key code.
    if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
    const { key } = event;
    if (key === "Escape") {
      releaseTab.current = true;
      return;
    }
    if (key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      const element = event.currentTarget;
      if (releaseTab.current) {
        releaseTab.current = false;
        // The shell keeps Tab from walking the page, so the move is made here.
        focusBeside(element, event.shiftKey ? -1 : 1);
        return;
      }
      const edit = indentCode(
        element.value,
        element.selectionStart,
        element.selectionEnd,
        event.shiftKey,
      );
      if (edit !== null) applyCodeEdit(element, edit);
      return;
    }
    // Holding Shift on the way to Shift+Tab keeps the way out armed.
    if (!["Shift", "Control", "Alt", "Meta"].includes(key)) releaseTab.current = false;
    if (key === "Enter" && (event.metaKey || event.ctrlKey) && !event.altKey) {
      event.preventDefault();
      event.currentTarget.form?.requestSubmit();
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (codeMode) {
      onCodeKeyDown(event);
      return;
    }
    if (commands.open) {
      const plainTab =
        event.key === "Tab" && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey;
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        commands.step(event.key === "ArrowDown" ? 1 : -1);
        return;
      }
      if ((event.key === "Enter" && !event.shiftKey) || plainTab) {
        if (event.nativeEvent.isComposing) return;
        event.preventDefault();
        const item = commands.selected;
        if (item !== undefined) chooseCommand(item, plainTab ? "complete" : "use");
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        // The list takes this Escape, so nothing beneath it acts on it too.
        event.stopPropagation();
        commands.close();
        return;
      }
    }
    if (mentionsShown) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        mentions.step(event.key === "ArrowDown" ? 1 : -1);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        if (event.nativeEvent.isComposing) return;
        event.preventDefault();
        if (mentions.selected !== undefined) chooseMention(mentions.selected);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        // The list takes this Escape, so nothing beneath it (a panel, the
        // drawers) acts on it too.
        event.stopPropagation();
        mentions.dismiss();
        return;
      }
    }
    if ((event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) {
      const key = event.key.toLowerCase();
      const marker = key === "b" ? "**" : key === "i" ? "*" : null;
      if (marker !== null) {
        event.preventDefault();
        applyFormat(marker);
        return;
      }
    }
    if (event.key !== "Enter" || event.shiftKey) return;
    // An IME candidate list also closes on Enter; sending there would eat the
    // word the user was still composing.
    if (event.nativeEvent.isComposing) return;
    event.preventDefault();
    event.currentTarget.form?.requestSubmit();
  };

  /**
   * The phone's sheet: tiles, then the commands for the message and the
   * server, each where the registry places it. Formatting is among the
   * message's, because its button is not in the phone's row, and so is
   * whatever adds to a message without a tile of its own, as GIFs do.
   */
  const sheetContent = (base: CommandBase): { tiles: SheetEntry[]; groups: SheetGroup[] } => {
    const context = contextFor(base);
    const entry = (command: ComposerCommand): SheetEntry[] => {
      const availability = command.available(context);
      if (!availability.shown) return [];
      const name = t(command.name);
      return [
        {
          id: command.id,
          label: t(command.label),
          detail: `/${name}`,
          icon: command.icon,
          names: [name, command.alias, t(command.label)],
          disabled: availability.blocked !== undefined,
          // A tap is the whole choice: only what cannot run without its
          // arguments (who to remove) asks for them in the field.
          onSelect: () => {
            if (command.args.some((arg) => !arg.optional)) beginCommand(command, name, base, "plus");
            else runCommand(command, NO_INPUT, base);
          },
        },
      ];
    };
    const rows = (group: ComposerCommand["group"]) =>
      COMMANDS.filter((command) => command.sheet === "row" && command.group === group).flatMap(entry);
    const formatting: SheetEntry = {
      id: "format",
      label: t("composer.format"),
      detail: t("composer.commands.formatDetail"),
      icon: "textFormat",
      names: [t("composer.format")],
      onSelect: () => setFormatOpen(true),
    };
    const [first, ...rest] = rows("message");
    const added = rows("add");
    return {
      tiles: COMMANDS.flatMap((command) => (typeof command.sheet === "object" ? [{ command, tile: command.sheet.tile }] : []))
        .sort((left, right) => left.tile - right.tile)
        .flatMap(({ command }) => entry(command)),
      groups: [
        {
          id: "message",
          label: t("composer.commands.group.message"),
          entries: first === undefined ? [...added, formatting] : [first, ...added, formatting, ...rest],
        },
        {
          id: "server",
          label: t("composer.commands.group.server"),
          entries: rows("server"),
        },
      ],
    };
  };

  const cancelPress = () => {
    if (press.current !== null) window.clearTimeout(press.current);
    press.current = null;
  };
  /** The menu stands over the send button, whatever opened it. */
  const openSendMenu = (button: HTMLElement) => {
    const box = button.getBoundingClientRect();
    setSendMenu({ x: box.right, y: box.top });
  };
  /**
   * "Wyslij pozniej" opens on a right click, on the keyboard's menu key or
   * Shift+F10 (the browser reports both as a context menu), and on a finger
   * held on the button.
   */
  const sendLaterHandlers = {
    onContextMenu: (event: MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      // A touch browser reports the long press itself too: one menu, and no send after it.
      if (press.current !== null) swallowClick.current = true;
      cancelPress();
      if (canSend) openSendMenu(event.currentTarget);
    },
    onPointerDown: (event: PointerEvent<HTMLButtonElement>) => {
      if (event.pointerType === "mouse") return;
      const button = event.currentTarget;
      cancelPress();
      press.current = window.setTimeout(() => {
        press.current = null;
        swallowClick.current = true;
        openSendMenu(button);
      }, LONG_PRESS_MS);
    },
    onPointerUp: cancelPress,
    onPointerLeave: cancelPress,
    onPointerCancel: cancelPress,
    onClick: (event: MouseEvent<HTMLButtonElement>) => {
      if (!swallowClick.current) return;
      swallowClick.current = false;
      event.preventDefault();
    },
  };

  const prompt = placeholder ?? t("composer.toChannel", { channel: channelName });
  const noAttachments = t("composer.noPrivateAttachments");
  const voiceTitle = !attachmentsSupported
    ? noAttachments
    : hasPayload
      ? t("composer.voiceMustBeSeparate")
      : voiceUnavailable
        ? t("composer.recordingUnavailable")
        : undefined;
  const selectedMention = mentions.selected;
  const selectedCommand = commands.selected;
  const codeLines = codeMode && text.length <= CODE_MESSAGE_MAX_LENGTH ? countLines(text) : 0;
  // A paste of thousands of lines can be edited directly in the textarea.
  const highlightCode = codeLines > 0 && codeLines <= 5_000;
  const commandsOpenedHere = commands.mode === "plus" || sheetOpen;

  return (
    <form
      ref={formElement}
      className="composer"
      onSubmit={(event) => {
        // The send button stands for the command while one is in the field.
        if (active !== null) {
          event.preventDefault();
          runActive();
          return;
        }
        void submit(event);
      }}
    >
      {/* On top of the box rather than inside it: the field keeps its own
          height and the answer stays visible while a long message is typed. */}
      {replyingTo === null ? null : (
        <ComposerReplyBar
          authorName={replyingTo.authorName}
          preview={replyingTo.preview}
          onCancel={onCancelReply}
          t={t}
        />
      )}
      <ComposerChips
        chips={chips}
        onRemove={onRemoveChip}
        onRetry={onRetryChip}
        onReorder={onReorderChips}
      />
      <input
        ref={filePicker}
        type="file"
        multiple
        hidden
        onChange={(event) => {
          const files = Array.from(event.currentTarget.files ?? []);
          event.currentTarget.value = "";
          if (files.length > 0) onAttach?.(files);
        }}
      />
      {voiceOpen && !voiceUnavailable ? (
        <div className="composer-box voice-mode">
          <VoiceRecorder
            openMicrophone={openVoiceMessageMicrophone}
            onCancel={() => {
              setVoiceOpen(false);
              requestAnimationFrame(() => field.current?.focus());
            }}
            onSend={onSendVoice}
          />
        </div>
      ) : (
        <>
        {scheduling === undefined || scheduling.waiting.count === 0 ? null : (
          <ScheduledBar
            count={scheduling.waiting.count}
            next={scheduling.waiting.next}
            place={scheduling.place}
            now={now}
            zone={zone}
            locale={locale}
            onShow={scheduling.onShowScheduled}
            t={t}
          />
        )}
        {timeOverBox && sendAt !== null ? (
          <div className={scheduleCss.chipStrip}>
            <WhenChip
              label={whenLabel(sendAt, now, zone, locale, t)}
              stale={timeProblem(sendAt, now) !== null}
              onRemove={() => setSendAt(null)}
              t={t}
            />
          </div>
        ) : null}
        <div
          className={[
            "composer-box",
            codeMode ? codeCss.box : null,
            sendAt === null ? null : scheduleCss.scheduled,
          ].filter(Boolean).join(" ")}
          // The phone's row keys on these: the plus is the commands', and the
          // microphone gives way to send once there is something to send.
          data-commands={codeMode ? undefined : ""}
          data-payload={hasPayload ? "" : undefined}
        >
          {active !== null ? null : commands.open ? (
            <CommandList
              ref={commandPop}
              id={commandListId}
              hintId={commandHintId}
              items={commands.items}
              grouped={commands.grouped}
              selectedIndex={commands.selectedIndex}
              onChoose={(item) => chooseCommand(item, "use")}
              onHover={commands.select}
              t={t}
            />
          ) : mentionsShown ? (
            <MentionList
              id={mentionListId}
              label={t("composer.mentionPeople")}
              candidates={mentions.matches}
              selectedIndex={mentions.selectedIndex}
              onChoose={chooseMention}
            />
          ) : null}
          {codeMode ? (
            // Code mode turns the box itself into an editor, with its head
            // inside it; there is no command to give a code block.
            <ComposerCodeHead
              language={codeLanguage}
              onLanguageChange={setCodeLanguage}
              guess={codeGuess}
              room={codeRoom ?? CODE_MESSAGE_MAX_LENGTH}
              onDisable={automaticCode ? undefined : () => {
                switchCodeMode(false);
                // The head and its button are gone; the field is where the
                // keyboard was working.
                requestAnimationFrame(() => field.current?.focus());
              }}
              onPicked={() => field.current?.focus()}
              helpId={codeHelpId}
              counterId={codeCounterId}
              locale={locale}
              t={t}
            />
          ) : (
            // The left slot is the way to everything the message can carry
            // or lead to: files, GIFs, code, the server's own actions.
            <button
              ref={commandTrigger}
              type="button"
              className="comp-lead"
              aria-label={t("composer.commands.open")}
              aria-haspopup={sheetOpen ? "dialog" : "listbox"}
              aria-expanded={commandsOpenedHere}
              aria-controls={commands.mode === "plus" && commands.open ? commandListId : undefined}
              title={t("composer.commands.open")}
              onClick={onCommandsButton}
            >
              <Icon name={commandsOpenedHere ? "x" : "plus"} size={18} />
            </button>
          )}
          {/* The chosen time sits in the field, before the words it is for. */}
          {timeOverBox || sendAt === null || active !== null ? null : (
            <WhenChip
              label={whenLabel(sendAt, now, zone, locale, t)}
              stale={timeProblem(sendAt, now) !== null}
              onRemove={() => {
                setSendAt(null);
                field.current?.focus();
              }}
              t={t}
            />
          )}
          {active === null ? null : (
            <CommandArgs
              active={active}
              problems={Object.fromEntries(
                Object.entries(activeProblems).map(([id, key]) => [id, t(key)]),
              )}
              people={host.people ?? NO_PEOPLE}
              locale={locale}
              onText={commands.setArgument}
              onPerson={commands.setPerson}
              onFocusArgument={commands.focusArgument}
              onRun={runActive}
              onBack={() => leaveCommand(true)}
              t={t}
            />
          )}
          {/* One wrapper in both modes, so the textarea stays the same element,
              with its focus and undo history, when code mode is switched. Out
              of code mode it draws no box of its own. While a command is in
              the field it waits, hidden, with the draft the command leaves. */}
          <div
            className={codeMode ? codeCss.field : codeCss.plainField}
            hidden={active !== null}
            data-composing={codeMode && composing ? "" : undefined}
            data-plain-code={codeMode && !highlightCode ? "" : undefined}
            style={
              highlightCode
                ? ({ "--digits": String(codeLines).length } as CSSProperties)
                : undefined
            }
          >
            {highlightCode ? (
              <ComposerCodeLayers field={field} text={text} language={codeHighlight} />
            ) : null}
            <textarea
              readOnly={loadingDraft}
              ref={field}
              className={
                commands.open && commands.slash !== null && text === `/${commands.query}`
                  ? commandCss.typedCommand
                  : undefined
              }
              rows={1}
              value={text}
              placeholder={codeMode ? t("composer.codePlaceholder") : prompt}
              aria-label={prompt}
              aria-describedby={codeMode ? `${codeHelpId} ${codeCounterId}` : undefined}
              wrap={codeMode ? "off" : undefined}
              spellCheck={codeMode ? false : undefined}
              autoCapitalize={codeMode ? "off" : undefined}
              autoCorrect={codeMode ? "off" : undefined}
              // Outside code mode the field always offers a list: the
              // commands after a slash, and people after "@".
              role={codeMode ? undefined : "combobox"}
              aria-autocomplete={codeMode ? undefined : "list"}
              aria-expanded={codeMode ? undefined : commands.open || mentionsShown}
              aria-controls={
                commands.open ? commandListId : mentionsShown ? mentionListId : undefined
              }
              aria-activedescendant={
                commands.open && selectedCommand !== undefined
                  ? commandOptionId(commandListId, selectedCommand)
                  : mentionsShown && selectedMention !== undefined
                    ? mentionOptionId(mentionListId, selectedMention)
                    : undefined
              }
              onChange={(event) => {
                const value = event.target.value;
                const at = event.target.selectionStart;
                clearSendError();
                typing();
                if (takeTypedCommand(value, at)) return;
                setText(value);
                trackCaret(at);
              }}
              onSelect={(event) => trackCaret(event.currentTarget.selectionStart)}
              onPaste={(event) => {
                if (!attachmentsSupported || onAttach === undefined) return;
                const files = Array.from(event.clipboardData.files);
                if (files.length === 0) return;
                // Screenshots and copied files belong to the attachment queue. Do
                // not also paste the browser's fallback filename into the draft.
                event.preventDefault();
                onAttach(files);
              }}
              onKeyDown={onKeyDown}
              onCompositionStart={() => setComposing(true)}
              onCompositionEnd={() => setComposing(false)}
              onBlur={() => {
                releaseTab.current = false;
              }}
            />
          </div>
          {/* Code mode keeps the phone's disclosure for its own row; outside
              it, the plus and the sheet hold what the disclosure held. */}
          {codeMode ? (
            <button
              ref={mobileActionsTrigger}
              type="button"
              className="attach comp-mobile-more"
              aria-label={t("composer.moreActions")}
              aria-controls={mobileActionsId}
              aria-expanded={mobileActionsOpen}
              onClick={() => setMobileActionsOpen((open) => !open)}
            >
              <Icon name={mobileActionsOpen ? "x" : "plus"} size={18} />
            </button>
          ) : null}
          <div
            ref={mobileActions}
            id={mobileActionsId}
            className={codeMode && mobileActionsOpen ? "comp-actions mobile-open" : "comp-actions"}
            role="toolbar"
            aria-label={t("composer.moreActions")}
            hidden={active !== null}
          >
            {/* Formatting shapes the words being written; a code block has none. */}
            {codeMode ? null : (
              <button
                ref={formatTrigger}
                type="button"
                className="attach comp-format"
                aria-label={t("composer.format")}
                aria-haspopup="true"
                aria-expanded={formatOpen}
                title={t("composer.format")}
                onClick={() => setFormatOpen((open) => !open)}
              >
                <Icon name="textFormat" size={18} />
              </button>
            )}
            <button
              type="button"
              className="attach comp-code-trigger"
              aria-label={t("composer.sendAsCode")}
              aria-pressed={codeMode}
              title={t("composer.sendAsCode")}
              onClick={() => switchCodeMode(!codeMode)}
            >
              <Icon name="code" size={18} />
            </button>
            {/* In code mode the plus stays here, for files only: the slot on
                the left is the code's, and a slash there is code. */}
            {codeMode ? (
              <button
                type="button"
                className="attach comp-x"
                aria-label={t("composer.addAttachment")}
                disabled={!attachmentsSupported || attachmentsFull}
                title={
                  attachmentsFull
                    ? t("composer.attachmentLimit")
                    : attachmentsSupported
                      ? t("composer.addAttachment")
                      : noAttachments
                }
                onClick={() => {
                  setMobileActionsOpen(false);
                  openPicker(ANY_ATTACHMENT);
                }}
              >
                <Icon name="plus" size={18} />
              </button>
            ) : null}
            {onGif === undefined ? null : (
              <GifBrowser ref={gifBrowser} onSelect={onGif} />
            )}
            {/* Not in code mode, as drawn: an emoji is not code. */}
            {codeMode ? null : (
              <button
                ref={emojiTrigger}
                type="button"
                className="attach"
                aria-label={t("emoji.open")}
                aria-haspopup="dialog"
                aria-expanded={emojiOpen}
                onPointerDown={(event) => setEmojiAutoFocus(event.pointerType === "mouse")}
                onClick={(event) => {
                  // A click from a keyboard or assistive technology has no
                  // pointer event before it and carries detail 0.
                  if (event.detail === 0) setEmojiAutoFocus(true);
                  // Read before the click moves focus off the field.
                  caret.current = field.current?.selectionStart ?? null;
                  setEmojiOpen(!emojiOpen);
                  if (!emojiOpen) setMobileActionsOpen(false);
                }}
              >
                <Icon name="smiley" size={18} />
              </button>
            )}
            <button
              type="button"
              className="attach comp-x comp-voice"
              aria-label={t("composer.recordVoice")}
              disabled={!attachmentsSupported || hasPayload || sending || voiceUnavailable}
              title={voiceTitle}
              onClick={() => {
                setMobileActionsOpen(false);
                setVoiceOpen(true);
              }}
            >
              <Icon name="mic" size={18} />
            </button>
          </div>
          {active !== null ? (
            <>
              <button
                type="button"
                className={commandCss.cancel}
                aria-label={t("composer.commands.cancel")}
                title={t("composer.commands.cancel")}
                onClick={() => leaveCommand(false)}
              >
                <Icon name="x" size={16} />
              </button>
              {/* The send button's place: it runs the command, or asks first. */}
              <button
                type="submit"
                className={`send ${commandCss.run}`}
                disabled={!activeReady}
                aria-label={t("composer.commands.run")}
                title={t("composer.commands.run")}
              >
                <Icon name="send" size={15} />
              </button>
            </>
          ) : (
            // Disabled is not only a guard: `.send:disabled` collapses the
            // button to zero width, which is how the empty composer looks.
            <button
              type="submit"
              className={[
                "send",
                sending ? "sending" : hasPayload && attachmentsBlocked ? "waiting" : null,
                sendAt === null ? null : scheduleCss.later,
                scheduling === undefined ? null : scheduleCss.pressable,
              ].filter(Boolean).join(" ")}
              disabled={!canSend}
              aria-label={
                sending
                  ? t("composer.sending")
                  : attachmentsBlocked
                    ? t("composer.waitForAttachments")
                    : sendAt === null
                      ? t("composer.send")
                      : undefined
              }
              aria-haspopup={scheduling === undefined ? undefined : "menu"}
              aria-expanded={scheduling === undefined ? undefined : sendMenu !== null}
              aria-busy={sending}
              {...(scheduling === undefined ? {} : sendLaterHandlers)}
            >
              <Icon name={sendAt === null ? "send" : "clock"} size={15} />
              {sendAt === null ? null : <span>{t("composer.schedule.send")}</span>}
            </button>
          )}
        </div>
        </>
      )}
      {/* Outside `.composer-box`, so they are positioned against the form that
          `.fmt-pop` and `.emo-pop` anchor to; inside, the box's own rounding
          would clip them. */}
      {formatOpen ? (
        <ComposerFormatPanel panelRef={formatPanel} onFormat={applyFormat} t={t} />
      ) : null}
      {emojiOpen ? (
        <div className="emo-pop" ref={emojiPanel} role="dialog" aria-label={t("emoji.search")}>
          {/* The panel stays open after a pick: sending three emoji in a row is
              the common case. A Unicode emoji goes in as itself, which every
              reader renders; an uploaded one goes in as `:code:`, the only way a
              message body can carry a picture that lives on the server. */}
          <EmojiPicker
            customEmoji={customEmoji}
            autoFocus={emojiAutoFocus}
            onPick={(entry) =>
              insertEmoji(
                entry.imageUrl === undefined ? entry.glyph : `:${entry.shortcode}:`,
                emojiAutoFocus,
              )
            }
            onClose={() => {
              setEmojiOpen(false);
              emojiTrigger.current?.focus();
            }}
          />
        </div>
      ) : null}
      {sendMenu === null || scheduling === undefined ? null : (
        <ContextMenu
          at={sendMenu}
          label={t("composer.schedule.menu")}
          titled
          entries={[
            ...schedulePresets(now, zone).map((preset) => ({
              id: preset.id,
              ...presetRow(preset, now, zone, locale, t),
              onSelect: () => chooseTime(preset.at, caret.current ?? text.length),
            })),
            "separator" as const,
            {
              id: "pick",
              label: t("composer.schedule.pick"),
              trailingIcon: "calendar" as const,
              onSelect: () => setPicker({ initial: sendAt }),
            },
          ]}
          onClose={() => {
            setSendMenu(null);
            // Closed without a choice: back to the field rather than nowhere.
            requestAnimationFrame(() => {
              if (document.activeElement === document.body) field.current?.focus();
            });
          }}
        />
      )}
      {picker === null || scheduling === undefined ? null : (
        <SchedulePicker
          now={now}
          zone={zone}
          locale={locale}
          initial={picker.initial}
          onCancel={() => {
            setPicker(null);
            selectLater(caret.current ?? text.length);
          }}
          onConfirm={(at) => {
            setPicker(null);
            chooseTime(at, caret.current ?? text.length);
          }}
          t={t}
        />
      )}
      {sheet === null ? null : (
        <CommandSheet
          tiles={sheet.tiles}
          groups={sheet.groups}
          returnFocusTo={commandTrigger}
          onClose={() => setSheet(null)}
          t={t}
        />
      )}
      {sendError === null ? null : (
        <p className="composer-error" role="alert">
          {sendError}
        </p>
      )}
    </form>
  );
}
