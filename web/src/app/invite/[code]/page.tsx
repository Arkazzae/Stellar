import { InviteClient } from "./InviteClient";

/**
 * The code this route is rendered ahead of time for. It names no invite (the
 * server's codes are hexadecimal), and the page reads the real code from the
 * address, so this one rendering can be served for every `/invite/<code>`
 * and links already shared keep working without a page per code.
 */
const RENDERED_CODE = "_";

export function generateStaticParams() {
  return [{ code: RENDERED_CODE }];
}

export default function InvitePage() {
  return <InviteClient />;
}
