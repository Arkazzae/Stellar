/** Only these app entries can be carried across a sign-in. */
export function launchEntry(params: Pick<URLSearchParams, "get">): string | null {
  const id = params.get("notice");
  if (id && /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/i.test(id)) return "?notice=" + id;
  const share = params.get("share");
  if (share && (share === "error" || /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/i.test(share))) return "?share=" + share;
  const view = params.get("view");
  return view === "messages" || view === "inbox" ? "?view=" + view : null;
}
