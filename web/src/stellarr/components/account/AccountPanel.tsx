import { useEffect, useRef, useState, type MouseEvent } from "react";

import { Icon } from "@stellarr/icons";
import { AnchoredCard } from "@/components/primitives/AnchoredCard";
import { PresenceDot } from "@/components/primitives/Avatar";
import { EmojiPicker } from "@/components/emoji/EmojiPicker";
import { CustomStatusText } from "@/components/profile/ProfileBio";
import { graphemes } from "@/lib/messages/parts";
import type { CustomEmoji, Presence } from "@/ports";
import { PRESENCE_VALUES, SELF_PRESENCE_LABEL } from "@/ports";
import { useLanguage } from "@/state/hooks";

/**
 * A status stays a glanceable line; the longer text belongs to the bio. The
 * server counts UTF-16 units, as `maxLength` does, emoji included.
 */
const STATUS_MAX_LENGTH = 128;
/** The count stays out of the way until the limit is close. */
const COUNT_FROM = 100;
/** How long the saved tick stands in for the field's action. */
const SAVED_FLASH_MS = 1500;

const SHORTCODE = /^:([a-zA-Z0-9_-]{2,64}):/;
/** Flags are pairs of regional indicators, not pictographs. */
const PICTOGRAPH = /^(?:\p{Extended_Pictographic}|\p{Regional_Indicator})/u;
const LINE_BREAKS = /[\r\n]+/g;

interface StatusParts {
  emoji: string | undefined;
  text: string;
}

/**
 * The emoji a status starts with. A whole grapheme, so a flag or a ZWJ
 * sequence stays one emoji. A `:shortcode:` counts only when the server has
 * it: otherwise there is no picture for the slot, and it is words.
 */
function leadingEmoji(
  status: string,
  customEmoji: readonly CustomEmoji[],
): string | undefined {
  const code = SHORTCODE.exec(status);
  if (code !== null) {
    const known = customEmoji.some(
      ({ shortcode }) => shortcode.replace(/^:|:$/g, "") === code[1],
    );
    return known ? code[0] : undefined;
  }
  const first = graphemes(status)[0];
  return first !== undefined && PICTOGRAPH.test(first) ? first : undefined;
}

/** The emoji for the field's slot, and the words for the field itself. */
function splitStatus(
  status: string,
  customEmoji: readonly CustomEmoji[],
): StatusParts {
  const emoji = leadingEmoji(status, customEmoji);
  return emoji === undefined
    ? { emoji, text: status }
    : { emoji, text: status.slice(emoji.length).trimStart() };
}

/** Room left for the words once the emoji and its space are counted. */
function roomForText(emoji: string | undefined): number {
  return STATUS_MAX_LENGTH - (emoji === undefined ? 0 : emoji.length + 1);
}

/**
 * One string on the wire: the emoji, a space, the words. An emoji picked or
 * typed into a full line takes its room from the end of the words, without
 * leaving half of a surrogate pair behind.
 */
function joinStatus({ emoji, text }: StatusParts): string {
  const room = roomForText(emoji);
  const words =
    text.length <= room ? text : text.slice(0, room).replace(/[\uD800-\uDBFF]$/, "");
  return emoji === undefined ? words : `${emoji} ${words}`;
}

export interface AccountPanelProps {
  presence: Presence;
  customStatus: string;
  customEmoji?: readonly CustomEmoji[];
  onSetPresence: (presence: Presence) => void;
  onSaveCustomStatus: (customStatus: string) => void;
  onLogout: () => void;
}

interface StatusEditorProps {
  customStatus: string;
  customEmoji: readonly CustomEmoji[];
  onSave: (customStatus: string) => void;
}

/** The short line under the user's name: one field, its emoji and one action. */
function StatusEditor({ customStatus, customEmoji, onSave }: StatusEditorProps) {
  const { t } = useLanguage();
  const [draft, setDraft] = useState(customStatus);
  const [saved, setSaved] = useState(false);
  const [pickerAnchor, setPickerAnchor] = useState<HTMLElement | null>(null);
  const [pickerAutoFocus, setPickerAutoFocus] = useState(true);
  const savedTimer = useRef<number | undefined>(undefined);
  const field = useRef<HTMLTextAreaElement>(null);
  const focusFieldAfterPick = useRef(false);

  // The status changed somewhere else, such as in another tab: start the
  // draft over from it instead of keeping an edit that no longer applies.
  const [seenStatus, setSeenStatus] = useState(customStatus);
  if (seenStatus !== customStatus) {
    setSeenStatus(customStatus);
    setDraft(customStatus);
  }

  useEffect(() => () => window.clearTimeout(savedTimer.current), []);

  // The closing card hands focus back to the slot in its own cleanup, which
  // runs before this effect, so the field is focused after it.
  useEffect(() => {
    const element = field.current;
    if (pickerAnchor !== null || !focusFieldAfterPick.current || element === null) return;
    focusFieldAfterPick.current = false;
    element.focus();
    element.setSelectionRange(element.value.length, element.value.length);
  }, [pickerAnchor]);

  const { emoji, text } = splitStatus(draft, customEmoji);
  const length = joinStatus({ emoji, text }).length;
  const dirty = draft.trim() !== customStatus.trim();
  const showsSaved = saved && !dirty;

  const save = () => {
    if (!dirty) return;
    onSave(draft.trim());
    setSaved(true);
    window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => setSaved(false), SAVED_FLASH_MS);
  };

  /** Re-reads the line, so an emoji typed at its start moves to the slot. */
  const setText = (next: string) =>
    setDraft(joinStatus(splitStatus(joinStatus({ emoji, text: next }), customEmoji)));

  /** Puts the emoji first, or takes it off again when it already leads. */
  const toggleLeading = (picked: string) =>
    setDraft(picked === emoji ? text : joinStatus({ emoji: picked, text }));

  // Saving and clearing replace the button that was pressed. A pointer never
  // focuses it, so the caret stays in the field; a keyboard press (detail 0)
  // is sent back to the field instead of to the page body.
  const keepFocus = (event: MouseEvent) => event.preventDefault();
  const refocus = (event: MouseEvent) => {
    if (event.detail === 0) field.current?.focus();
  };

  const togglePicker = (button: HTMLElement) =>
    setPickerAnchor((anchor) => (anchor === null ? button : null));

  const emojiLabel = t(emoji === undefined ? "account.addEmoji" : "account.chooseEmoji");

  return (
    <>
      <form
        className="acc-status"
        onSubmit={(event) => {
          event.preventDefault();
          save();
        }}
      >
        <button
          type="button"
          className="acc-status-emoji"
          aria-label={emojiLabel}
          title={emojiLabel}
          aria-haspopup="dialog"
          aria-expanded={pickerAnchor !== null}
          // Touch and pen keep the search field unfocused, so opening the
          // catalogue does not raise the software keyboard. A click with no
          // pointer behind it (detail 0) is the keyboard, which wants focus.
          onPointerDown={(event) =>
            setPickerAutoFocus(event.pointerType === "mouse")
          }
          onClick={(event) => {
            if (event.detail === 0) setPickerAutoFocus(true);
            togglePicker(event.currentTarget);
          }}
        >
          {emoji === undefined ? (
            <Icon name="smiley" size={16} />
          ) : (
            <CustomStatusText text={emoji} customEmoji={customEmoji} />
          )}
        </button>

        {/* The copy behind the field is what sizes it, so the line grows with
            its words even while the panel is hidden and nothing can be
            measured. */}
        <span className="acc-status-text" data-text={text}>
          <label className="sr-only" htmlFor="acc-status-field">
            {t("account.yourStatus")}
          </label>
          <textarea
            ref={field}
            id="acc-status-field"
            value={text}
            rows={1}
            maxLength={roomForText(emoji)}
            placeholder={t("account.statusPlaceholder")}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing) return;
              if (event.key === "Enter") {
                // A status is one line; Enter is the save.
                event.preventDefault();
                save();
              } else if (
                event.key === "Backspace" &&
                emoji !== undefined &&
                event.currentTarget.selectionStart === 0 &&
                event.currentTarget.selectionEnd === 0
              ) {
                event.preventDefault();
                setDraft(text);
              }
            }}
            onChange={(event) =>
              setText(event.target.value.replace(LINE_BREAKS, " "))
            }
          />
        </span>

        {dirty ? (
          <button
            type="submit"
            className="acc-status-act save"
            aria-label={t("common.save")}
            title={t("common.save")}
            onMouseDown={keepFocus}
            onClick={refocus}
          >
            <Icon name="tick" size={17} />
          </button>
        ) : showsSaved ? (
          // Not a button: a second click of a double click lands here and
          // must not clear what the first one saved.
          <span className="acc-status-act saved" title={t("common.saved")}>
            <Icon name="tick" size={17} />
          </span>
        ) : customStatus.trim() === "" ? null : (
          <button
            type="button"
            className="acc-status-act clear"
            aria-label={t("account.clearStatus")}
            title={t("account.clearStatus")}
            onMouseDown={keepFocus}
            onClick={(event) => {
              onSave("");
              refocus(event);
            }}
          >
            <Icon name="x" size={15} />
          </button>
        )}
      </form>

      {length >= COUNT_FROM ? (
        <div className="acc-status-count">
          {length}/{STATUS_MAX_LENGTH}
        </div>
      ) : null}

      {pickerAnchor === null ? null : (
        <AnchoredCard
          anchor={pickerAnchor}
          onClose={() => setPickerAnchor(null)}
          label={t("account.chooseEmoji")}
          className="acc-emoji-card"
          autoFocus={pickerAutoFocus}
        >
          <EmojiPicker
            customEmoji={customEmoji}
            autoFocus={pickerAutoFocus}
            onPick={(entry) => {
              toggleLeading(
                entry.imageUrl === undefined
                  ? entry.glyph
                  : `:${entry.shortcode}:`,
              );
              // Back to the words, so Enter saves. Not after a touch or pen
              // pick: focusing the field would raise the software keyboard.
              focusFieldAfterPick.current = pickerAutoFocus;
              setPickerAnchor(null);
            }}
            onClose={() => setPickerAnchor(null)}
          />
        </AnchoredCard>
      )}
    </>
  );
}

/**
 * The compact menu for frequent account actions: a short status, presence and
 * sign-out. The full profile editor lives in AccountPane, behind the row's gear.
 */
export function AccountPanel({
  presence,
  customStatus,
  customEmoji = [],
  onSetPresence,
  onSaveCustomStatus,
  onLogout,
}: AccountPanelProps) {
  const { t } = useLanguage();

  return (
    <>
      <StatusEditor
        customStatus={customStatus}
        customEmoji={customEmoji}
        onSave={onSaveCustomStatus}
      />

      <hr className="acc-sep" />

      <div role="group" aria-label={t("account.yourAvailability")}>
        {PRESENCE_VALUES.map((value) => (
          <button
            key={value}
            type="button"
            className={value === presence ? "acc-item on" : "acc-item"}
            aria-pressed={value === presence}
            onClick={() => onSetPresence(value)}
          >
            <span className="acc-item-icon">
              <PresenceDot presence={value} />
            </span>
            <span className="acc-item-label">
              {t(SELF_PRESENCE_LABEL[value])}
            </span>
            {value === presence ? <Icon name="check" size={16} /> : null}
          </button>
        ))}
      </div>

      <hr className="acc-sep" />

      <button type="button" className="acc-item acc-out" onClick={onLogout}>
        <span className="acc-item-icon">
          <Icon name="signout" size={16} />
        </span>
        <span className="acc-item-label">{t("common.logout")}</span>
      </button>
    </>
  );
}
