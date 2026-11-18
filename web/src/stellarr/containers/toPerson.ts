import type { PersonVM } from "@/components/view-models";
import { initials } from "@/lib/format/format";
import type { User } from "@/ports";

/** A user as the messages side shows a person. */
export function toPerson(user: User): PersonVM {
  return {
    id: user.id,
    displayName: user.displayName,
    initials: initials(user.displayName),
    presence: user.presence,
    ...(user.avatarUrl === undefined ? {} : { avatarUrl: user.avatarUrl }),
  };
}
