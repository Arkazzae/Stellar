import type { AccountField, FormIssue } from "@/components/auth/AccountFields";
import type { MessageKey, Translator } from "@/i18n";
import { ApiError } from "@/lib/api/errors";

/** What a registration can be refused for, by the key the server names it with. */
export type RegistrationField = AccountField | "setupCode" | "inviteCode" | "server" | "channels";

/**
 * Each key of the server's validation problem, in the order the forms show
 * their fields, with the sentence that says how to fix it. The server words
 * its own messages in the instance's language, which may not be the
 * reader's, so the key picks the sentence and the server's text is not shown.
 * The form has already checked what it can, so a refused login or address
 * is most likely taken.
 */
const FIELDS: readonly [RegistrationField, MessageKey][] = [
  ["setupCode", "auth.setup.codeRefused"],
  ["displayName", "auth.register.displayNameInvalid"],
  ["username", "auth.register.usernameTaken"],
  ["email", "auth.register.emailRefused"],
  ["password", "auth.password.length"],
  ["inviteCode", "auth.register.inviteRefused"],
  ["server", "auth.setup.serverRefused"],
  ["channels", "auth.setup.invalidChannel"],
];

export function registrationIssue(cause: unknown, t: Translator): FormIssue<RegistrationField> {
  if (cause instanceof ApiError) {
    const refused = cause.fields ?? {};
    const found = FIELDS.find(([field]) => field in refused);
    if (found !== undefined) return { field: found[0], message: t(found[1]) };
    if (cause.status === 429) return { field: null, message: t("auth.register.tooMany") };
  }
  return { field: null, message: t("auth.register.failed") };
}
