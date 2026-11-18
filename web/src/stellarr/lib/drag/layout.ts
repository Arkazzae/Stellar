/**
 * The DOM side of a sort: reading where things sit without their transforms,
 * and moving whole blocks of the page with transforms only.
 */

/**
 * Where `node` starts inside `container`, in content pixels: neither its
 * transforms nor the container's scrolling change it, so a sort can measure
 * again in the middle of a drag. The container has to be positioned, so the
 * offsets of what is inside are taken from it.
 */
export function layoutTop(node: HTMLElement, container: HTMLElement): number {
  let top = 0;
  for (
    let at: HTMLElement | null = node;
    at !== null && at !== container;
    at = at.offsetParent instanceof HTMLElement ? at.offsetParent : null
  ) {
    top += at.offsetTop;
  }
  return top;
}

/** One element of the page as measured, which a sort can move as a whole. */
export interface Block {
  node: HTMLElement;
  top: number;
  height: number;
  /** The blocks inside, for an element that holds a list; null for anything else. */
  children: Block[] | null;
  /** Holds the carried item, whose own transform must not be added to. */
  holdsCarried: boolean;
}

/**
 * Every element under `container` that a sort may have to move, measured
 * once. Elements that hold one of `lists` are opened up, down to the items;
 * everything else, a caption or a list of another kind, is one block.
 */
export function measureBlocks(
  container: HTMLElement,
  lists: readonly HTMLElement[],
  carried: HTMLElement,
): Block[] {
  const opened = new Set<Element>();
  for (const list of lists) {
    for (let node: Element | null = list; node !== null && node !== container; node = node.parentElement) {
      opened.add(node);
    }
  }
  const blocksIn = (parent: Element): Block[] =>
    [...parent.children].flatMap((node) =>
      node instanceof HTMLElement
        ? [
            {
              node,
              top: layoutTop(node, container),
              height: node.offsetHeight,
              children: opened.has(node) ? blocksIn(node) : null,
              holdsCarried: node.contains(carried),
            },
          ]
        : [],
    );
  return blocksIn(container);
}

/**
 * How far each block has to move, as few and as large blocks as possible: a
 * block moves as one unless the shift changes inside it, or it holds the
 * carried item, and then its children move instead.
 */
export function planShifts(
  blocks: readonly Block[],
  carried: HTMLElement,
  breaks: readonly number[],
  shiftAt: (y: number) => number,
): Map<HTMLElement, number> {
  const moves = new Map<HTMLElement, number>();
  const visit = (level: readonly Block[]) => {
    for (const block of level) {
      if (block.node === carried) continue;
      const straddles = breaks.some((y) => block.top < y && y < block.top + block.height);
      if (block.children !== null && (block.holdsCarried || straddles)) {
        visit(block.children);
        continue;
      }
      const by = shiftAt(block.top);
      if (by !== 0) moves.set(block.node, by);
    }
  };
  visit(blocks);
  return moves;
}

/**
 * Writes the shifts of `next` over those of `applied`, touching only what
 * changed. A block keeps `data-drag` until the sort ends, so one that has to
 * go back eases back instead of jumping.
 */
export function writeShifts(
  applied: Map<HTMLElement, number>,
  next: Map<HTMLElement, number>,
): Map<HTMLElement, number> {
  for (const node of applied.keys()) {
    if (!next.has(node)) node.style.transform = "";
  }
  for (const [node, by] of next) {
    if (applied.get(node) === by) continue;
    node.dataset.drag = "shifted";
    node.style.transform = `translateY(${by}px)`;
  }
  return next;
}
