import { chordsEqual, isModifierOf, pressMatches, type KeyChord } from "./keybinds";

/** A single key, without Ctrl, Alt or Meta: one a text field keeps for itself. */
function isSingleKey(chord: KeyChord): boolean {
  return !chord.ctrl && !chord.alt && !chord.meta;
}

/**
 * Where a key press belongs to what is being typed: a field, a list to pick
 * from, or editable text.
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target instanceof HTMLElement && target.isContentEditable) return true;
  return target.closest("input, textarea, select, [contenteditable]:not([contenteditable='false'])") !== null;
}

/**
 * What became of the talk key: held, let go, or cut. A cut is a release
 * nobody chose (the window lost focus, the page was hidden, the key was
 * rebound) and closes the microphone at once instead of after its tail.
 */
export type TalkChange = "held" | "released" | "cut";

/**
 * The talk key's state: held or not, from the key events the page receives.
 *
 * Holds only while it may act at all (push to talk chosen, a call joined, no
 * shortcut being recorded) and only on a real press: auto-repeat never starts
 * it, so a key still down from before, after the window comes back, has to be
 * pressed again. A single key, function keys included, does nothing while a
 * text field has focus, which keeps the key for typing; one held with Ctrl,
 * Alt or Meta acts everywhere.
 */
export class TalkKey {
  private binding: KeyChord | null = null;
  private active = false;
  private down = false;

  constructor(private readonly changed: (change: TalkChange) => void) {}

  get held(): boolean {
    return this.down;
  }

  /**
   * Which chord is the talk key, and whether it may act now. Any change while
   * it is held cuts it: a key rebound or a mode left while talking must not
   * leave the microphone open.
   */
  configure(binding: KeyChord | null, active: boolean): void {
    const same =
      active === this.active &&
      (binding === this.binding ||
        (binding !== null && this.binding !== null && chordsEqual(binding, this.binding)));
    this.binding = binding;
    this.active = active;
    if (!same) this.cut();
  }

  /**
   * A key went down. True when it was the talk key's; its default action is
   * then prevented, so a held Space does not scroll and a held letter does
   * not press the focused control.
   */
  press(event: KeyboardEvent): boolean {
    const binding = this.binding;
    if (!this.active || binding === null) return false;
    if (this.down) {
      // The held key repeating: still the talk key's, and nothing new.
      if (event.code !== binding.code) return false;
      event.preventDefault();
      return true;
    }
    if (event.repeat || !pressMatches(binding, event)) return false;
    if (isSingleKey(binding) && isEditableTarget(event.target)) return false;
    event.preventDefault();
    this.set(true, "held");
    return true;
  }

  /**
   * A key came up. The talk key's own key lets it go, by its code alone: by
   * then the modifiers may well have changed. So does any modifier the chord
   * is held with, because macOS sends no key-up for other keys while Cmd is
   * down. True when it was the talk key's own key, whose default is then
   * prevented too: Space presses a focused button when it comes up.
   */
  lift(event: KeyboardEvent): boolean {
    const binding = this.binding;
    if (!this.down || binding === null) return false;
    if (event.code === binding.code) {
      event.preventDefault();
      this.set(false, "released");
      return true;
    }
    if (isModifierOf(binding, event.code)) this.set(false, "released");
    return false;
  }

  /** Lets go at once, without the tail. */
  cut(): void {
    this.set(false, "cut");
  }

  private set(down: boolean, change: TalkChange): void {
    if (down === this.down) return;
    this.down = down;
    this.changed(change);
  }
}

/**
 * Feeds `key` the page's key events, and cuts it whenever its release could
 * go unseen: the window losing focus (to another window, or to an activity's
 * frame, whose keys this page never receives), the page being hidden, or the
 * page being put away. Returns the way to stop, which cuts it as well.
 *
 * The listeners run in the capture phase, as the other shortcuts' do, so the
 * talk key is seen wherever the focus is, and a press it takes reaches nothing
 * else.
 */
export function listenForTalkKey(
  key: TalkKey,
  scope: { document: Document; window: Window } = { document, window },
): () => void {
  const { document: page, window: view } = scope;
  const onKeyDown = (event: KeyboardEvent) => {
    if (key.press(event)) event.stopImmediatePropagation();
  };
  const onKeyUp = (event: KeyboardEvent) => {
    if (key.lift(event)) event.stopImmediatePropagation();
  };
  const cut = () => key.cut();
  const onVisibility = () => {
    if (page.visibilityState === "hidden") key.cut();
  };

  page.addEventListener("keydown", onKeyDown, true);
  page.addEventListener("keyup", onKeyUp, true);
  page.addEventListener("visibilitychange", onVisibility);
  view.addEventListener("blur", cut);
  view.addEventListener("pagehide", cut);
  return () => {
    page.removeEventListener("keydown", onKeyDown, true);
    page.removeEventListener("keyup", onKeyUp, true);
    page.removeEventListener("visibilitychange", onVisibility);
    view.removeEventListener("blur", cut);
    view.removeEventListener("pagehide", cut);
    key.cut();
  };
}
