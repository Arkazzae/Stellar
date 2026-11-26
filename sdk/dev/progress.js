import { isDeepStrictEqual } from "node:util";
import { fail } from "./room.js";

// Called after the room has checked the active frame, revision range and byte limit.
export function previewProgress(stored, action, commit) {
  const current = stored ?? { revision: 0, data: null };
  if (action.op === "loadProgress") return current;
  if (action.expected === current.revision) {
    const next = { revision: current.revision + 1, data: action.data };
    commit(next);
    return { revision: next.revision };
  }
  if (current.revision === action.expected + 1 && isDeepStrictEqual(current.data, action.data))
    return { revision: current.revision };
  fail(409, "Progress changed. Load the latest revision.");
}
