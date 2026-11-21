import type { CSSProperties } from "react";

import { Icon } from "@stellarr/icons";
import type { Role } from "@/ports";

/** A role's mark: a shield in its colour, the owner's a crown. */
export function RoleMark({ role, size }: { role: Pick<Role, "color" | "isOwner">; size: number }) {
  return (
    <span className="ss-role-ico" style={{ "--rc": role.color } as CSSProperties} aria-hidden="true">
      <Icon name={role.isOwner ? "crown" : "shield"} size={size} />
    </span>
  );
}
