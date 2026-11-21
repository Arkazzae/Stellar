/**
 * What React rendered in each commit of one root, by component name.
 *
 * Mount the tree under `<Profiler id="…" onRender={log.commit}>`, render it,
 * then `log.watch(container)`: from there on every commit is read back from
 * the committed fibers, the way React's own devtools tell a render from a
 * reuse. React marks a fiber whose render it ran with its PerformedWork flag
 * and clears the flag on every copy, and a fiber whose children are still the
 * ones the previous commit left bailed out together with everything under it.
 */

/** The fields of a fiber this reads. */
interface Fiber {
  tag: number;
  type: { displayName?: string; name?: string; render?: { name?: string } } | null;
  flags: number;
  child: Fiber | null;
  sibling: Fiber | null;
  alternate: Fiber | null;
  stateNode: { current: Fiber } | null;
}

const PERFORMED_WORK = 1;
/** Function, class, forwardRef and simple memo components; a memo wrapper's inner fiber is its own. */
const COMPONENTS = new Set([0, 1, 11, 15]);

function nameOf(fiber: Fiber): string {
  const type = fiber.type;
  return type?.displayName ?? type?.name ?? type?.render?.name ?? "Anonymous";
}

function collect(fiber: Fiber, into: string[]): void {
  const before = fiber.alternate;
  if (before !== null && fiber.child === before.child) return;
  for (let child = fiber.child; child !== null; child = child.sibling) {
    const rendered = child.alternate === null || (child.flags & PERFORMED_WORK) !== 0;
    if (rendered && COMPONENTS.has(child.tag)) into.push(nameOf(child));
    collect(child, into);
  }
}

export interface RenderLog {
  /** The Profiler's `onRender`. */
  commit: () => void;
  /** Starts recording the root mounted in `container`; the mount itself is not recorded. */
  watch: (container: HTMLElement) => void;
  /** The commits since the last call, each a list of the components it rendered. */
  take: () => string[][];
}

export function renderLog(): RenderLog {
  let root: Fiber | null = null;
  let commits: string[][] = [];
  return {
    commit: () => {
      if (root === null) return;
      const rendered: string[] = [];
      const current = root.stateNode?.current;
      if (current !== undefined) collect(current, rendered);
      commits.push(rendered);
    },
    watch: (container) => {
      const key = Object.keys(container).find((name) => name.startsWith("__reactContainer$"));
      if (key === undefined) throw new Error("no React root in this container");
      root = (container as unknown as Record<string, Fiber>)[key] ?? null;
      commits = [];
    },
    take: () => {
      const taken = commits;
      commits = [];
      return taken;
    },
  };
}
