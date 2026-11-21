import type { ConfirmSubject } from "@/components/primitives/ConfirmSubject";

/**
 * Someone as a question card shows them: their face, their name and their
 * handle when it is known. On its own, apart from `confirmations.ts`, because
 * a provider the first page loads asks about a person too.
 */
export function personSubject(person: {
  id: string;
  displayName: string;
  avatarUrl?: string;
  username?: string;
}): Extract<ConfirmSubject, { kind: "person" }> {
  return {
    kind: "person",
    id: person.id,
    name: person.displayName,
    ...(person.avatarUrl === undefined ? {} : { avatarUrl: person.avatarUrl }),
    ...(person.username === undefined || person.username === "" ? {} : { handle: person.username }),
  };
}
