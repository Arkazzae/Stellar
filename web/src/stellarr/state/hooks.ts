/**
 * The hooks that containers and components use.
 *
 * Each provider exports its hook under a long `use*Context` name; this file
 * gathers the short names in the one place the rest of the app imports from.
 * It is the only barrel in `src/stellarr/state`, and it re-exports hooks and their value
 * types, never the providers, which are composed once in `StellarrApp` and
 * in `AppContainer`.
 */

export {
  useAccountContext as useAccount,
  type AccountValue,
} from "./AccountProvider";
export { useChatContext as useChat, type ChatValue } from "./ChatProvider";
export {
  useDirectoryContext as useDirectory,
  type DirectoryValue,
} from "./DirectoryProvider";
export { useEmoji, type EmojiValue } from "./EmojiProvider";
export {
  useEncryptionContext as useEncryption,
  type EncryptionDialog,
  type EncryptionValue,
} from "./EncryptionProvider";
export {
  useFriendsContext as useFriends,
  type FriendsValue,
} from "./FriendsProvider";
export {
  useInstanceSetupContext as useInstanceSetup,
  type InstanceSetupValue,
} from "./InstanceSetupProvider";
export {
  useKeybindContext as useKeybinds,
  type KeybindValue,
} from "./KeybindProvider";
export {
  useLanguage,
  type InterfaceLocale,
  type LanguageSettings,
  type LanguageValue,
  type TranslationTargetLanguage,
} from "./LanguageProvider";
export {
  useNotificationContext as useNotifications,
  type NotificationValue,
} from "./NotificationProvider";
export { usePorts, useRuntime } from "./PortsProvider";
export {
  useProfileContext as useProfile,
  type ProfileValue,
} from "./ProfileProvider";
export {
  useSessionContext as useSession,
  type SessionPhase,
  type SessionValue,
} from "./SessionProvider";
export { useTheme } from "./ThemeProvider";
export {
  useUiContext as useUi,
  type ModalKey,
  type Space,
  type UiValue,
} from "./UiProvider";
export {
  useVoiceContext as useVoice,
  useVoiceLiveContext as useVoiceLive,
  type VoiceLive,
  type VoiceValue,
} from "./VoiceProvider";
