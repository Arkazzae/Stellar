import { useEffect, useLayoutEffect, type RefObject } from "react";

/**
 * The textarea stops growing at eight rows. `composer.css` caps it at
 * `calc(1.6em * 8)`; this is the same number in pixels (15px font-size times
 * the 1.6 line-height declared there), because `scrollHeight` has to be
 * clamped before it is written back as a height.
 */
const MAX_ROWS = 8;
const ROW_PX = 24;
const MAX_HEIGHT = MAX_ROWS * ROW_PX;

function fit(element: HTMLTextAreaElement | null, maxHeight: number) {
  if (element === null) return;
  element.style.height = "auto";
  // What the box has beyond its client area, such as the bar under a field
  // that scrolls sideways, is added back, or that bar would cover the last row.
  const chrome = element.offsetHeight - element.clientHeight;
  element.style.height = `${Math.min(element.scrollHeight + chrome, maxHeight)}px`;
}

/**
 * Keeps a textarea as tall as its text, up to eight rows, or to `maxHeight`
 * pixels when the field is set in another size.
 *
 * `scope` names the place the field is used in. It is a dependency on purpose:
 * leaving a narrower thread must discard the height measured for that layout
 * even if React reuses the same field before its text changes.
 *
 * A `hidden` field, such as the one waiting behind a command's arguments,
 * measures as nothing, so it is left alone and fitted again when it shows.
 */
export function useAutosizedField(
  field: RefObject<HTMLTextAreaElement | null>,
  text: string,
  scope: string | null,
  maxHeight = MAX_HEIGHT,
  hidden = false,
) {
  // Measure-then-set has to happen before paint, or the box visibly jumps a
  // frame behind the caret.
  useLayoutEffect(() => {
    if (!hidden) fit(field.current, maxHeight);
  }, [field, text, scope, maxHeight, hidden]);

  // A thread can also be expanded or restored without the draft changing.
  // Refit only when the form changes width; observing height as well would
  // make our own adjustment feed back into the observer. The form is watched
  // rather than the field's parent, which may draw no box of its own, and
  // rather than the field, which a voice message unmounts and puts back.
  useEffect(() => {
    const element = field.current;
    const box = element?.closest("form") ?? element?.parentElement;
    if (box == null || typeof ResizeObserver === "undefined") return;
    let width = box.getBoundingClientRect().width;
    const observer = new ResizeObserver(([entry]) => {
      if (entry === undefined || entry.contentRect.width === width) return;
      width = entry.contentRect.width;
      if (!hidden) fit(field.current, maxHeight);
    });
    observer.observe(box);
    return () => observer.disconnect();
  }, [field, scope, maxHeight, hidden]);
}
