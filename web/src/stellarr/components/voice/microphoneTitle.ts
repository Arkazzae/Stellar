import type { Translator } from "@/i18n";
import { keyChordParts, type KeyChord } from "@/lib/keybinds/keybinds";
import type { InputMode } from "@/ports";

/**
 * The microphone button's title, in the call's bar and in the dock alike:
 * what a click does, which is always the mute, and in push to talk also
 * which key opens the microphone, or that none does yet.
 */
export function microphoneTitle(
  t: Translator,
  muted: boolean,
  inputMode: InputMode,
  talkKey: KeyChord | null,
): string {
  const action = muted ? t("voiceBar.unmute") : t("voiceBar.mute");
  if (inputMode !== "pushToTalk") return action;
  return talkKey === null
    ? t("voiceBar.pushToTalkNoKey", { action })
    : t("voiceBar.pushToTalkTitle", { action, key: keyChordParts(talkKey).join("+") });
}
