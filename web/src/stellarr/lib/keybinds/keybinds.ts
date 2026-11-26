export const KEYBIND_STORAGE_KEY = "stellarr.keybinds.v1";

export const KEYBIND_ACTIONS = [
  "toggleMute",
  "toggleDeafen",
  "toggleCamera",
  "disconnect",
  "pushToTalk",
] as const;

export type KeybindAction = (typeof KEYBIND_ACTIONS)[number];

/**
 * The action that is held rather than pressed: the talk key, which opens the
 * microphone for as long as it is down. Every other action fires once per press.
 */
export const TALK_ACTION = "pushToTalk" satisfies KeybindAction;

/** The actions that fire once on a key press. */
export type ToggleAction = Exclude<KeybindAction, typeof TALK_ACTION>;

export interface KeyChord {
  code: string;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

export type Keybindings = Record<KeybindAction, KeyChord | null>;
export type KeybindValidation = "valid" | "needs-modifier" | "reserved";
export type KeybindRecordingError =
  | { kind: Exclude<KeybindValidation, "valid"> }
  | { kind: "conflict"; action: KeybindAction };

/** The shape `parseKeybindings` reads, here and on the account. */
export const KEYBIND_STORAGE_VERSION = 1;
const STORAGE_VERSION = KEYBIND_STORAGE_VERSION;

/** Keys that only describe a chord and cannot be its main key. */
const MODIFIER_CODES: ReadonlySet<string> = new Set([
  "AltLeft",
  "AltRight",
  "ControlLeft",
  "ControlRight",
  "MetaLeft",
  "MetaRight",
  "ShiftLeft",
  "ShiftRight",
]);

/** With Ctrl or Cmd these belong to the browser (zoom, find, tabs, ...) or to the app. */
const RESERVED_COMMAND_CODES: ReadonlySet<string> = new Set([
  "Digit0",
  "Equal",
  "KeyF",
  "KeyK",
  "KeyL",
  "KeyN",
  "KeyO",
  "KeyP",
  "KeyR",
  "KeyS",
  "KeyT",
  "KeyW",
  "Minus",
]);

/** Ctrl or Cmd with Shift opens the browser's developer tools and inspectors. */
const RESERVED_SHIFT_COMMAND_CODES: ReadonlySet<string> = new Set([
  "KeyC",
  "KeyD",
  "KeyI",
]);

/**
 * Keys the talk key may not be without Ctrl, Alt or Meta: held down as the
 * talk key, each would stop doing its own job everywhere outside a text
 * field, which is to move the focus (Tab, the arrows), press the focused
 * control (Enter, Space), close what is open (Escape) or scroll (Home, End,
 * Page Up, Page Down).
 */
const RESERVED_PLAIN_TALK_CODES: ReadonlySet<string> = new Set([
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "End",
  "Enter",
  "Escape",
  "Home",
  "NumpadEnter",
  "PageDown",
  "PageUp",
  "Space",
  "Tab",
]);

/**
 * Keys that cannot be the talk key at all, because a press does not reliably
 * come with its release. A lock key reports going down when it switches on and
 * up when it switches off, on two separate presses on macOS, which would leave
 * the microphone open in between; the menu and print keys give the page only
 * one half of the press on some systems.
 */
const RESERVED_TALK_CODES: ReadonlySet<string> = new Set([
  "CapsLock",
  "ContextMenu",
  "NumLock",
  "PrintScreen",
  "ScrollLock",
]);

function ctrlShift(code: string): KeyChord {
  return Object.freeze({ code, ctrl: true, alt: false, shift: true, meta: false });
}

export const DEFAULT_KEYBINDINGS: Keybindings = Object.freeze({
  toggleMute: ctrlShift("KeyM"),
  toggleDeafen: ctrlShift("KeyE"),
  toggleCamera: null,
  disconnect: ctrlShift("KeyH"),
  // Unbound until the person picks one: any key chosen for them would be
  // one they also need for something else.
  pushToTalk: null,
});

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Reads one stored binding. `null` is a deliberately cleared action, while
 * `undefined` means the stored value is unusable and the default applies.
 */
function parseStoredChord(value: unknown, action: KeybindAction): KeyChord | null | undefined {
  if (value === null) return null;
  if (!isRecord(value)) return undefined;

  const { code, ctrl, alt, shift, meta } = value;
  if (
    typeof code !== "string" ||
    code === "" ||
    typeof ctrl !== "boolean" ||
    typeof alt !== "boolean" ||
    typeof shift !== "boolean" ||
    typeof meta !== "boolean"
  ) {
    return undefined;
  }

  const chord: KeyChord = { code, ctrl, alt, shift, meta };
  return validateKeyChord(chord, action) === "valid" ? chord : undefined;
}

/** Restores bindings from untrusted storage, never leaving two actions on one chord. */
export function parseKeybindings(value: unknown): Keybindings {
  const stored =
    isRecord(value) && value["version"] === STORAGE_VERSION
      ? value["bindings"]
      : undefined;
  const result: Keybindings = { ...DEFAULT_KEYBINDINGS };
  if (!isRecord(stored)) return result;

  const taken: KeyChord[] = [];
  for (const action of KEYBIND_ACTIONS) {
    const parsed = parseStoredChord(stored[action], action);
    const candidate = parsed === undefined ? DEFAULT_KEYBINDINGS[action] : parsed;
    const duplicate =
      candidate !== null && taken.some((chord) => chordsEqual(chord, candidate));

    result[action] = duplicate ? null : candidate;
    if (!duplicate && candidate !== null) taken.push(candidate);
  }
  return result;
}

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    // Reading the property itself throws when site data is blocked.
    return null;
  }
}

export function readStoredKeybindings(
  storage: Storage | null = safeStorage(),
): Keybindings {
  try {
    const raw = storage?.getItem(KEYBIND_STORAGE_KEY) ?? null;
    if (raw !== null) return parseKeybindings(JSON.parse(raw));
  } catch {
    // Unreadable or corrupt storage falls through to the defaults.
  }
  return { ...DEFAULT_KEYBINDINGS };
}

export function writeStoredKeybindings(
  bindings: Keybindings,
  storage: Storage | null = safeStorage(),
): void {
  try {
    storage?.setItem(
      KEYBIND_STORAGE_KEY,
      JSON.stringify({ version: STORAGE_VERSION, bindings }),
    );
  } catch {
    // A private or locked-down profile may reject storage. The session keeps
    // the binding in memory, so the control still works until the page closes.
  }
}

/** The chord a key press makes, or `null` while only modifiers are held. */
export function chordFromEvent(event: KeyboardEvent): KeyChord | null {
  if (event.code === "" || MODIFIER_CODES.has(event.code)) return null;
  return {
    code: event.code,
    ctrl: event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey,
    meta: event.metaKey,
  };
}

function isFunctionKey(code: string): boolean {
  return /^F(?:[1-9]|1\d|2[0-4])$/.test(code);
}

/**
 * True for a chord that types: no Ctrl, Alt or Meta, and not a function key.
 * Shift alone still types, a capital. Only the talk key may be one.
 */
function isPlainChord(chord: KeyChord): boolean {
  return !chord.ctrl && !chord.alt && !chord.meta && !isFunctionKey(chord.code);
}

/**
 * Whether `chord` may be bound to `action`. Without an action the rules of the
 * pressed actions apply, which need a modifier.
 */
export function validateKeyChord(chord: KeyChord, action?: KeybindAction): KeybindValidation {
  const isCommand = chord.ctrl || chord.meta;

  if (action === TALK_ACTION) {
    // The talk key is held, not typed: a plain key is the usual choice, and
    // in a text field it types as before (see `TalkKey`).
    if (RESERVED_TALK_CODES.has(chord.code)) return "reserved";
    if (!isCommand && !chord.alt && RESERVED_PLAIN_TALK_CODES.has(chord.code)) {
      return "reserved";
    }
  } else if (isPlainChord(chord)) {
    // Shift alone is not enough: it would fire on ordinary typing.
    return "needs-modifier";
  }

  if (isCommand && !chord.alt) {
    if (RESERVED_COMMAND_CODES.has(chord.code)) return "reserved";
    if (chord.shift && RESERVED_SHIFT_COMMAND_CODES.has(chord.code)) {
      return "reserved";
    }
  }
  return "valid";
}

export function chordsEqual(left: KeyChord, right: KeyChord): boolean {
  return (
    left.code === right.code &&
    left.ctrl === right.ctrl &&
    left.alt === right.alt &&
    left.shift === right.shift &&
    left.meta === right.meta
  );
}

/** The chord a key press makes, unless it is not one a shortcut may answer. */
function pressedChord(event: KeyboardEvent): KeyChord | null {
  // AltGr reports Ctrl+Alt on Windows, and a composing IME is mid-word.
  if (event.isComposing || event.getModifierState("AltGraph")) return null;
  return chordFromEvent(event);
}

/** The action a key press triggers, if any. */
export function actionForKeyboardEvent(
  bindings: Keybindings,
  event: KeyboardEvent,
): KeybindAction | null {
  const pressed = pressedChord(event);
  if (pressed === null) return null;

  return (
    KEYBIND_ACTIONS.find((action) => {
      const bound = bindings[action];
      return bound !== null && chordsEqual(bound, pressed);
    }) ?? null
  );
}

/** Whether this key press is `chord`, by the same rules `actionForKeyboardEvent` uses. */
export function pressMatches(chord: KeyChord, event: KeyboardEvent): boolean {
  const pressed = pressedChord(event);
  return pressed !== null && chordsEqual(chord, pressed);
}

const MODIFIER_CODES_OF: Readonly<Record<"ctrl" | "alt" | "shift" | "meta", readonly string[]>> = {
  ctrl: ["ControlLeft", "ControlRight"],
  alt: ["AltLeft", "AltRight"],
  shift: ["ShiftLeft", "ShiftRight"],
  meta: ["MetaLeft", "MetaRight"],
};

/** Whether `code` is one of the modifiers `chord` is held with. */
export function isModifierOf(chord: KeyChord, code: string): boolean {
  return (["ctrl", "alt", "shift", "meta"] as const).some(
    (modifier) => chord[modifier] && MODIFIER_CODES_OF[modifier].includes(code),
  );
}

const KEY_LABELS: Readonly<Record<string, string>> = {
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  Backquote: "`",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Comma: ",",
  Equal: "=",
  Minus: "-",
  PageDown: "Page Down",
  PageUp: "Page Up",
  Period: ".",
  Quote: "'",
  Semicolon: ";",
  Slash: "/",
};

function keyLabel(code: string): string {
  const character = /^(?:Key|Digit)([A-Z0-9])$/.exec(code)?.[1];
  if (character !== undefined) return character;
  if (code.startsWith("Numpad")) return `Num ${code.slice("Numpad".length)}`;
  return KEY_LABELS[code] ?? code;
}

/** The chord as separate labels, in the order a keyboard legend shows them. */
export function keyChordParts(chord: KeyChord): string[] {
  const modifiers: [boolean, string][] = [
    [chord.ctrl, "Ctrl"],
    [chord.alt, "Alt"],
    [chord.shift, "Shift"],
    [chord.meta, "Meta"],
  ];
  return [
    ...modifiers.filter(([held]) => held).map(([, label]) => label),
    keyLabel(chord.code),
  ];
}
