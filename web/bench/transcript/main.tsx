import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { useLayoutEffect, useState } from "react";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { Transcript, type TranscriptSectionVM } from "@/components/room/transcript/Transcript";
import { loadCatalog } from "@/i18n";
import { documentLocale } from "@/i18n/defaultLocale";
import type { Message } from "@/ports";

import "./bench.css";
import "../../src/stellarr/styles/globals.css";

import {
  createSectionsProducer,
  expectedRows,
  makeMessages,
  markerFor,
  withMarker,
} from "./fixture";

interface View {
  sections: TranscriptSectionVM[];
  editing: { id: string; original: string } | null;
}

interface Anchor {
  id: string;
  top: number;
  scrollTop: number;
  scrollHeight: number;
}

const root = createRoot(document.getElementById("bench-root")!);
// The transcript translates as it renders, as in the app, which renders only
// once its language's catalog is here. Fetched now, awaited before timing.
const catalog = loadCatalog(documentLocale());
let setView: ((view: View) => void) | null = null;
let jumpTo: ((messageId: string) => void) | null = null;
let messages: Message[] = [];
let marker = { index: 0, text: "" };
let saved: Anchor | null = null;
let started = false;
let currentSections: TranscriptSectionVM[] = [];
let editingState: View["editing"] = null;
let toolbarEntry: HTMLElement | null = null;
let produce: ((messages: readonly Message[]) => TranscriptSectionVM[]) | null = null;

const noop = () => {};
const attachments = { onOpenImage: noop, onDownloadFile: noop, onPlayVideo: noop, onOpenEmbed: noop, onCopyCode: noop };
let lastAction: { messageId: string; action: string; at: number } | null = null;
let lastReact: { messageId: string; emoji: string } | null = null;
let lastEdit: { messageId: string; text: string; at: number } | null = null;
let lastCancel = false;

function now(): number {
  return performance.now();
}

function nextFrame(): Promise<number> {
  return new Promise((resolve) => requestAnimationFrame(resolve));
}

async function frames(count: number): Promise<void> {
  for (let index = 0; index < count; index += 1) await nextFrame();
}

async function waitForLayout(maxFrames = 120): Promise<{ frames: number; ms: number; stable: boolean }> {
  const start = now();
  let previous = "";
  let stableFrames = 0;
  let count = 0;
  while (count < maxFrames) {
    await nextFrame();
    count += 1;
    const scroller = scrollElement();
    const sample = `${scroller.scrollHeight}:${scroller.scrollTop}:${scroller.clientHeight}`;
    if (sample === previous) stableFrames += 1;
    else stableFrames = 0;
    previous = sample;
    if (stableFrames >= 2) break;
  }
  return { frames: count, ms: now() - start, stable: stableFrames >= 2 };
}

function scrollElement(): HTMLElement {
  const element = document.querySelector<HTMLElement>(".transcript");
  if (element === null) throw new Error("the transcript is not mounted");
  return element;
}

function rowElements(): HTMLElement[] {
  return [...scrollElement().querySelectorAll<HTMLElement>(".transcript-inner > [data-row-id]")];
}

function groupRows(): HTMLElement[] {
  return rowElements().filter((row) => row.querySelector(".msg-group") !== null);
}

function rectOf(row: HTMLElement): { top: number; bottom: number; height: number } {
  const box = row.getBoundingClientRect();
  const scroller = scrollElement().getBoundingClientRect();
  return { top: box.top - scroller.top, bottom: box.bottom - scroller.top, height: box.height };
}

function visibleGroup(fraction = 0.4): HTMLElement {
  const scroller = scrollElement();
  const target = scroller.clientHeight * fraction;
  const rows = groupRows();
  let best = rows[0];
  for (const row of rows) {
    const box = rectOf(row);
    if (box.bottom <= 0) continue;
    if (box.top > target) break;
    best = row;
  }
  if (best === undefined) throw new Error("no visible message row");
  return best;
}

function capture(fraction = 0.4): Anchor {
  const row = visibleGroup(fraction);
  const scroller = scrollElement();
  const box = rectOf(row);
  saved = {
    id: row.dataset.rowId ?? "",
    top: box.top,
    scrollTop: scroller.scrollTop,
    scrollHeight: scroller.scrollHeight,
  };
  return saved;
}

function measureSaved(): Record<string, unknown> {
  if (saved === null) throw new Error("no anchor was captured");
  const row = scrollElement().querySelector<HTMLElement>(`[data-row-id="${saved.id}"]`);
  if (row === null) throw new Error(`the anchor row ${saved.id} disappeared`);
  const scroller = scrollElement();
  const box = rectOf(row);
  return {
    id: saved.id,
    topBefore: saved.top,
    topAfter: box.top,
    delta: box.top - saved.top,
    scrollTopBefore: saved.scrollTop,
    scrollTopAfter: scroller.scrollTop,
    scrollHeightBefore: saved.scrollHeight,
    scrollHeightAfter: scroller.scrollHeight,
  };
}

function snapshot(): Record<string, unknown> {
  const scroller = scrollElement();
  const rows = rowElements();
  const groups = groupRows();
  const last = groups.at(-1);
  const lastBox = last === undefined ? null : last.getBoundingClientRect();
  const scrollerBox = scroller.getBoundingClientRect();
  return {
    rows: rows.length,
    groups: groups.length,
    entries: scroller.querySelectorAll(".msg-entry").length,
    elements: scroller.querySelectorAll("*").length,
    scrollHeight: scroller.scrollHeight,
    clientHeight: scroller.clientHeight,
    scrollTop: scroller.scrollTop,
    bottomGap: Math.max(0, scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop),
    lastRowVisible:
      lastBox !== null &&
      lastBox.bottom <= scrollerBox.bottom + 2 &&
      lastBox.bottom >= scrollerBox.top,
    lastRowBottom: lastBox === null ? null : lastBox.bottom - scrollerBox.top,
  };
}

function commit(view: View): void {
  if (setView === null) throw new Error("the fixture has not started");
  currentSections = view.sections;
  editingState = view.editing;
  flushSync(() => setView?.(view));
}

function Bench({ initial }: { initial: View }) {
  const [view, update] = useState<View>(initial);
  const messageCount = messages.length;
  useLayoutEffect(() => {
    setView = update;
    return () => {
      setView = null;
    };
  }, [update]);
  return (
    <ConfirmProvider>
      <div className="bench-view">
        <Transcript
          sections={view.sections}
          label="Benchmark transcript"
          editing={view.editing}
          attachments={attachments}
          onReact={(messageId, emoji) => {
            lastReact = { messageId, emoji };
          }}
          // The provider rebuilds these two whenever the message list changes
          // because they read the loaded messages. Keeping them new here is
          // deliberate: the benchmark must prove the transcript absorbs their
          // identity without redrawing every row.
          onAction={(messageId, action) => {
            lastAction = { messageId, action, at: messageCount };
          }}
          onEditSave={async (messageId, text) => {
            lastEdit = { messageId, text, at: messageCount };
          }}
          onEditCancel={() => {
            lastCancel = true;
          }}
          onJumpReady={(jump) => {
            jumpTo = jump;
          }}
        />
      </div>
    </ConfirmProvider>
  );
}

async function mount(count: number): Promise<Record<string, unknown>> {
  if (started) throw new Error("mount may only be called once");
  started = true;
  await catalog;
  messages = withMarker(makeMessages(0, count), count);
  marker = markerFor(count);
  produce = createSectionsProducer();
  const producerStart = now();
  const sections = produce(messages);
  const producerMs = now() - producerStart;
  currentSections = sections;
  const renderStart = now();
  flushSync(() => root.render(<Bench initial={{ sections, editing: null }} />));
  const commitMs = now() - renderStart;
  const layout = await waitForLayout();
  return {
    count,
    producerMs,
    commitMs,
    totalMs: now() - producerStart,
    layout,
    ...snapshot(),
    document: documentCounts(),
    style: styleInfo(),
    errors: [],
  };
}

/**
 * Every node in the document, and how much of it is message toolbars: a
 * toolbar is mostly inline icons, so it can be most of a message's DOM.
 */
function documentCounts(): Record<string, number> {
  let nodes = 0;
  const walker = document.createTreeWalker(document, NodeFilter.SHOW_ALL);
  while (walker.nextNode() !== null) nodes += 1;
  const toolbars = [...document.querySelectorAll(".msg-toolbar")];
  return {
    nodes,
    elements: document.getElementsByTagName("*").length,
    toolbars: toolbars.length,
    toolbarElements: toolbars.reduce(
      (sum, toolbar) => sum + 1 + toolbar.getElementsByTagName("*").length,
      0,
    ),
  };
}

async function append(count: number): Promise<Record<string, unknown>> {
  const before = snapshot();
  const producerStart = now();
  const next = makeMessages(messages.length, count, 23);
  messages = [...messages, ...next];
  if (produce === null) throw new Error("the fixture has not started");
  const sections = produce(messages);
  const producerMs = now() - producerStart;
  const commitStart = now();
  commit({ sections, editing: editingState });
  const commitMs = now() - commitStart;
  const layout = await waitForLayout();
  return {
    count,
    producerMs,
    commitMs,
    totalMs: now() - producerStart,
    layout,
    before,
    ...snapshot(),
  };
}

async function prepend(count: number): Promise<Record<string, unknown>> {
  const scroller = scrollElement();
  scroller.scrollTop = Math.round((scroller.scrollHeight - scroller.clientHeight) * 0.42);
  await waitForLayout();
  const anchor = capture(0.35);
  const before = snapshot();
  const producerStart = now();
  const older = makeMessages(-count, count, 31);
  messages = [...older, ...messages];
  if (produce === null) throw new Error("the fixture has not started");
  const sections = produce(messages);
  const producerMs = now() - producerStart;
  const commitStart = now();
  commit({ sections, editing: editingState });
  const commitMs = now() - commitStart;
  const layout = await waitForLayout();
  return {
    count,
    producerMs,
    commitMs,
    totalMs: now() - producerStart,
    anchor,
    layout,
    before,
    after: snapshot(),
    ...measureSaved(),
  };
}

async function growAbove(growth = 160): Promise<Record<string, unknown>> {
  const anchor = capture(0.4);
  const rows = rowElements();
  const index = rows.findIndex((row) => row.dataset.rowId === anchor.id);
  const target = rows[Math.max(0, index - 12)];
  if (target === undefined) throw new Error("no row above the anchor");
  const before = rectOf(target);
  target.style.minHeight = `${before.height + growth}px`;
  await frames(2);
  await new Promise((resolve) => setTimeout(resolve, 80));
  return {
    growth,
    target: target.dataset.rowId,
    targetContentVisibility: getComputedStyle(target).contentVisibility,
    targetHeightBefore: before.height,
    targetHeightAfter: rectOf(target).height,
    ...measureSaved(),
  };
}

async function growMediaAbove(growth = 120): Promise<Record<string, unknown>> {
  const anchor = capture(0.4);
  const rows = rowElements();
  const index = rows.findIndex((row) => row.dataset.rowId === anchor.id);
  let target: HTMLElement | null = null;
  let image: HTMLImageElement | null = null;
  for (let cursor = index - 1; cursor >= 0 && cursor >= index - 40; cursor -= 1) {
    const candidate = rows[cursor]?.querySelector<HTMLImageElement>(".att img");
    if (candidate !== null && candidate !== undefined) {
      target = rows[cursor] ?? null;
      image = candidate;
      break;
    }
  }
  if (target === null || image === null) return { skipped: true, reason: "no media row above" };
  const before = rectOf(target);
  image.style.height = `${before.height + growth}px`;
  image.style.maxHeight = "none";
  await frames(2);
  await new Promise((resolve) => setTimeout(resolve, 80));
  return {
    growth,
    target: target.dataset.rowId,
    targetContentVisibility: getComputedStyle(target).contentVisibility,
    targetHeightBefore: before.height,
    targetHeightAfter: rectOf(target).height,
    ...measureSaved(),
  };
}

async function scrollRun(steps = 80, delta = 180): Promise<Record<string, unknown>> {
  const scroller = scrollElement();
  scroller.scrollTop = Math.round((scroller.scrollHeight - scroller.clientHeight) * 0.2);
  await waitForLayout();
  const startHeight = scroller.scrollHeight;
  const longTasks: number[] = [];
  let observer: PerformanceObserver | null = null;
  try {
    observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) longTasks.push(entry.duration);
    });
    observer.observe({ entryTypes: ["longtask"] });
  } catch {
    observer = null;
  }
  const intervals: number[] = [];
  let previous = now();
  for (let index = 0; index < steps; index += 1) {
    await nextFrame();
    const at = now();
    intervals.push(at - previous);
    previous = at;
    scroller.scrollTop = Math.min(
      scroller.scrollHeight - scroller.clientHeight,
      scroller.scrollTop + delta,
    );
  }
  await waitForLayout();
  observer?.disconnect();
  const sorted = [...intervals].sort((a, b) => a - b);
  const at = (fraction: number) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))] ?? 0;
  return {
    steps,
    delta,
    distance: scroller.scrollTop,
    startHeight,
    endHeight: scroller.scrollHeight,
    meanMs: intervals.reduce((sum, value) => sum + value, 0) / Math.max(1, intervals.length),
    p50Ms: at(0.5),
    p95Ms: at(0.95),
    maxMs: sorted.at(-1) ?? 0,
    over25ms: intervals.filter((value) => value > 25).length,
    over50ms: intervals.filter((value) => value > 50).length,
    longTasks,
  };
}

async function waitForScroll(maxFrames = 240): Promise<{ frames: number; ms: number; stable: boolean }> {
  const start = now();
  const scroller = scrollElement();
  let previous = Number.NaN;
  let stable = 0;
  let count = 0;
  while (count < maxFrames) {
    await nextFrame();
    count += 1;
    if (Math.abs(scroller.scrollTop - previous) < 0.5) stable += 1;
    else stable = 0;
    previous = scroller.scrollTop;
    if (stable >= 4) return { frames: count, ms: now() - start, stable: true };
  }
  return { frames: count, ms: now() - start, stable: false };
}

async function jump(fraction: number): Promise<Record<string, unknown>> {
  const rows = groupRows();
  const target = rows[Math.min(rows.length - 1, Math.max(0, Math.floor(rows.length * fraction)))];
  if (target === undefined || jumpTo === null) throw new Error("the jump is not available");
  const id = target.dataset.rowId ?? "";
  jumpTo(id);
  const waited = await waitForScroll();
  const box = rectOf(target);
  return {
    fraction,
    id,
    waited,
    top: box.top,
    height: box.height,
    centreOffset: box.top + box.height / 2 - scrollElement().clientHeight / 2,
  };
}

async function replyJump(): Promise<Record<string, unknown>> {
  const rows = groupRows();
  for (let index = Math.floor(rows.length * 0.55); index < rows.length; index += 1) {
    const row = rows[index];
    if (row === undefined) continue;
    const button = row.querySelector<HTMLButtonElement>(".reply-ref");
    if (button === null) continue;
    const targetId = messages.find((message) => message.id === row.dataset.rowId)?.replyTo?.id;
    button.click();
    const waited = await waitForScroll();
    const target = targetId === undefined
      ? null
      : scrollElement().querySelector<HTMLElement>(`[data-row-id="${targetId}"]`);
    if (target === null) return { skipped: true, reason: "the answered row is not loaded" };
    const box = rectOf(target);
    return {
      from: row.dataset.rowId,
      id: targetId,
      waited,
      top: box.top,
      height: box.height,
      centreOffset: box.top + box.height / 2 - scrollElement().clientHeight / 2,
    };
  }
  return { skipped: true, reason: "no reply line below the middle" };
}

async function scrollUp(distance = 700): Promise<Record<string, unknown>> {
  const scroller = scrollElement();
  scroller.scrollTop = Math.max(0, scroller.scrollTop - distance);
  const layout = await waitForLayout();
  return { layout, ...snapshot() };
}

async function scrollTo(fraction: number): Promise<Record<string, unknown>> {
  const scroller = scrollElement();
  scroller.scrollTop = Math.round((scroller.scrollHeight - scroller.clientHeight) * fraction);
  const layout = await waitForLayout();
  return { layout, ...snapshot() };
}

/** The middle of a visible message entry, for a real pointer hover. */
function toolbarTarget(): Record<string, unknown> {
  const row = visibleGroup(0.5);
  const entry = row.querySelector<HTMLElement>(".msg-entry");
  if (entry === null) throw new Error("the visible row has no entry");
  toolbarEntry = entry;
  const box = entry.getBoundingClientRect();
  return { x: box.left + box.width / 2, y: box.top + box.height / 2, row: row.dataset.rowId };
}

/**
 * After the pointer has hovered the entry, the floating toolbar must be the
 * element on top at its own centre. `contain: layout` makes the row a stacking
 * context; this proves the toolbar still paints above the row it overlaps.
 */
function toolbarCheck(): Record<string, unknown> {
  if (toolbarEntry === null) throw new Error("no toolbar target was set");
  const toolbar = toolbarEntry.querySelector<HTMLElement>(".msg-toolbar");
  if (toolbar === null) throw new Error("the entry has no toolbar");
  const box = toolbar.getBoundingClientRect();
  const entryBox = toolbarEntry.getBoundingClientRect();
  const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
  const style = getComputedStyle(toolbar);
  return {
    opacity: style.opacity,
    pointerEvents: style.pointerEvents,
    hitToolbar: hit !== null && toolbar.contains(hit),
    hit: hit?.className ?? null,
    // Where it hangs on its message, which the style sheet puts 17 px above
    // the entry's top right corner.
    place: {
      top: Math.round((box.top - entryBox.top) * 10) / 10,
      right: Math.round((entryBox.right - box.right) * 10) / 10,
      width: Math.round(box.width * 10) / 10,
      height: Math.round(box.height * 10) / 10,
    },
  };
}

/**
 * The centre of a labelled button in the toolbar last made visible by
 * `toolbarTarget`, for a real pointer to travel to and press.
 */
function toolbarButtonPoint(label: string): Record<string, unknown> {
  if (toolbarEntry === null) throw new Error("no toolbar target was set");
  const button = toolbarEntry.querySelector<HTMLButtonElement>(
    `.msg-toolbar button[aria-label="${label}"]`,
  );
  if (button === null) throw new Error(`no toolbar action ${label}`);
  const box = button.getBoundingClientRect();
  return { label, x: box.left + box.width / 2, y: box.top + box.height / 2 };
}

/** What the pointer and focus leave of the toolbar on the entry `toolbarTarget` chose. */
function toolbarState(): Record<string, unknown> {
  if (toolbarEntry === null) throw new Error("no toolbar target was set");
  const toolbar = toolbarEntry.querySelector<HTMLElement>(".msg-toolbar");
  return {
    row: toolbarEntry.closest<HTMLElement>("[data-row-id]")?.dataset.rowId ?? null,
    hovered: toolbarEntry.matches(":hover"),
    focusWithin: toolbarEntry.matches(":focus-within"),
    toolbar: toolbar !== null,
    opacity: toolbar === null ? null : getComputedStyle(toolbar).opacity,
    picker: document.querySelector(".emo-pop-react") !== null,
    toolbars: document.querySelectorAll(".msg-toolbar").length,
    idle: idleToolbars(),
    lastReact,
  };
}

/** The centre of the open reaction picker and of its first emoji. */
function pickerPoints(): Record<string, unknown> {
  const panel = document.querySelector<HTMLElement>(".emo-pop-react .emo-pick");
  const option = document.querySelector<HTMLElement>('.emo-pop-react [role="option"]');
  if (panel === null || option === null) throw new Error("the reaction picker is not open");
  const centre = (element: HTMLElement) => {
    const box = element.getBoundingClientRect();
    return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
  };
  return { panel: centre(panel), option: centre(option), emoji: option.getAttribute("aria-label") };
}

/**
 * Toolbars in the document whose message nothing is using: not under the
 * pointer, not holding focus, and not the owner of an open picker or sheet.
 */
function idleToolbars(): { count: number; rows: string[] } {
  const rows: string[] = [];
  for (const toolbar of document.querySelectorAll<HTMLElement>(".msg-toolbar")) {
    const entry = toolbar.closest<HTMLElement>(".msg-entry");
    if (entry === null) continue;
    if (entry.matches(":hover") || entry.matches(":focus-within")) continue;
    if (entry.querySelector('[aria-expanded="true"]') !== null) continue;
    rows.push(entry.closest<HTMLElement>("[data-row-id]")?.dataset.rowId ?? "?");
  }
  return { count: rows.length, rows: rows.slice(0, 5) };
}

/** Messages under the pointer or holding focus that have no toolbar to show. */
function missingToolbars(): { count: number; rows: string[] } {
  const rows: string[] = [];
  for (const entry of document.querySelectorAll<HTMLElement>(".msg-entry")) {
    if (!entry.matches(":hover") && !entry.matches(":focus-within")) continue;
    if (entry.querySelector(".msg-toolbar") !== null) continue;
    rows.push(entry.closest<HTMLElement>("[data-row-id]")?.dataset.rowId ?? "?");
  }
  return { count: rows.length, rows: rows.slice(0, 5) };
}

/**
 * The phone's disclosure on a visible message, after a finger has tapped the
 * message: whether it is still shown and where it is, and whether the sheet
 * is open.
 */
function phoneActions(rowId: string): Record<string, unknown> {
  const entry = scrollElement().querySelector<HTMLElement>(`[data-row-id="${rowId}"] .msg-entry`);
  const more = entry?.querySelector<HTMLElement>(".msg-mobile-more") ?? null;
  if (entry === null || more === null) throw new Error(`no disclosure on ${rowId}`);
  const box = more.getBoundingClientRect();
  const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
  return {
    row: rowId,
    shown: getComputedStyle(more).display !== "none" && box.width > 0 && box.height > 0,
    hitDisclosure: hit !== null && more.contains(hit),
    x: box.left + box.width / 2,
    y: box.top + box.height / 2,
    sheet: document.querySelector(".msg-action-sheet") !== null,
    focusOnDisclosure: document.activeElement === more,
  };
}

/** A vertical line through the visible messages for a pointer sweep. */
function sweepPath(): Record<string, unknown> {
  const box = scrollElement().getBoundingClientRect();
  const visible = groupRows().filter((row) => {
    const rect = rectOf(row);
    return rect.bottom > 0 && rect.top < box.height;
  }).length;
  return {
    x: Math.round(box.left + box.width * 0.45),
    top: Math.round(box.top + 12),
    bottom: Math.round(box.bottom - 12),
    visibleGroups: visible,
  };
}

/** Puts the reader at a fraction of the history and names a point over the list for a wheel. */
async function wheelStart(fraction: number): Promise<Record<string, unknown>> {
  await scrollTo(fraction);
  const box = scrollElement().getBoundingClientRect();
  return {
    x: Math.round(box.left + box.width * 0.45),
    y: Math.round(box.top + box.height * 0.5),
    scrollTop: scrollElement().scrollTop,
  };
}

let frameSample: { intervals: number[]; running: boolean } | null = null;

/** Starts recording the interval between animation frames. */
function startFrames(): void {
  const sample = { intervals: [] as number[], running: true };
  frameSample = sample;
  let previous = now();
  const tick = () => {
    if (!sample.running) return;
    const at = now();
    sample.intervals.push(at - previous);
    previous = at;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

function stopFrames(): Record<string, number> {
  const sample = frameSample;
  frameSample = null;
  if (sample === null) return { frames: 0 };
  sample.running = false;
  // The first interval is the wait for the first frame, not a frame.
  const intervals = sample.intervals.slice(1);
  const sorted = [...intervals].sort((a, b) => a - b);
  const at = (fraction: number) =>
    sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))] ?? 0;
  return {
    frames: intervals.length,
    p50Ms: at(0.5),
    p95Ms: at(0.95),
    maxMs: sorted.at(-1) ?? 0,
    over33ms: intervals.filter((value) => value > 33).length,
  };
}

const FOCUSABLE = "a[href], button, input, textarea, select, [tabindex]";

/** Controls of a row that are not its messages' actions. */
function bodyFocusables(row: HTMLElement): HTMLElement[] {
  return [...row.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (element) =>
      element.closest(".msg-toolbar") === null &&
      !element.classList.contains("msg-mobile-more") &&
      element.tabIndex >= 0,
  );
}

/**
 * Where focus is, as a reader of the tab sequence would name it, whether it
 * shows its ring, and how far the transcript is scrolled with it there.
 */
function describeFocus(): Record<string, unknown> {
  const element = document.activeElement as HTMLElement | null;
  const top = Math.round(scrollElement().scrollTop);
  if (element === null || element === document.body) return { row: null, label: "body", action: false, ring: false, top };
  return {
    row: element.closest<HTMLElement>("[data-row-id]")?.dataset.rowId ?? null,
    label:
      element.getAttribute("aria-label") ??
      ((element.textContent ?? "").trim().slice(0, 40) || element.tagName),
    action: element.closest(".msg-toolbar") !== null,
    ring: element.matches(":focus-visible"),
    top,
  };
}

/**
 * Focuses the first action of a message, the way Tab arriving from above
 * would. With every toolbar mounted that is its first button; when one
 * control stands for a message's actions, it is that control.
 */
function focusFirstAction(row: HTMLElement): void {
  const entry = row.querySelector<HTMLElement>(".msg-entry");
  if (entry === null) throw new Error("the row has no message");
  const target =
    entry.querySelector<HTMLElement>(".msg-toolbar button") ??
    entry.querySelector<HTMLElement>(".msg-mobile-more");
  if (target === null) throw new Error("the message has no way to its actions");
  target.focus();
}

/**
 * Two neighbouring messages with nothing focusable but their actions, so the
 * Tab after the first one's last action can only be the second one's first,
 * and focus on the first one's first action.
 */
async function keyboardPair(fraction: number): Promise<Record<string, unknown>> {
  const rows = rowElements();
  for (let index = Math.floor(rows.length * fraction); index < rows.length - 1; index += 1) {
    const first = rows[index];
    const second = rows[index + 1];
    if (first === undefined || second === undefined) continue;
    const plain = (row: HTMLElement) =>
      row.querySelectorAll(".msg-entry").length === 1 && bodyFocusables(row).length === 0;
    if (!plain(first) || !plain(second)) continue;
    scrollElement().scrollTop = Math.max(0, scrollElement().scrollTop + rectOf(first).top - 300);
    await waitForLayout();
    focusFirstAction(first);
    await frames(1);
    return { first: first.dataset.rowId, second: second.dataset.rowId, focus: describeFocus() };
  }
  throw new Error("no two plain messages in a row");
}

/** Focus on the first action of the first message at or after a fraction of the history. */
async function keyboardSpan(fraction: number): Promise<Record<string, unknown>> {
  const rows = groupRows();
  const row = rows[Math.min(rows.length - 1, Math.floor(rows.length * fraction))];
  if (row === undefined) throw new Error("no message row");
  scrollElement().scrollTop = Math.max(0, scrollElement().scrollTop + rectOf(row).top - 200);
  await waitForLayout();
  focusFirstAction(row);
  await frames(1);
  return { row: row.dataset.rowId, focus: describeFocus() };
}

/** The middle of a message, for a pointer or a finger. */
function entryPoint(rowId: string): Record<string, unknown> {
  const entry = scrollElement().querySelector<HTMLElement>(`[data-row-id="${rowId}"] .msg-entry`);
  if (entry === null) throw new Error(`no message ${rowId}`);
  toolbarEntry = entry;
  const box = entry.getBoundingClientRect();
  return { x: box.left + box.width / 2, y: box.top + box.height / 2, row: rowId };
}

function styleInfo(): Record<string, unknown> {
  const rows = rowElements();
  const first = rows[0];
  const far = rows[Math.min(rows.length - 1, Math.floor(rows.length * 0.6))];
  const read = (element: HTMLElement | undefined) => {
    if (element === undefined) return null;
    const style = getComputedStyle(element);
    return {
      contentVisibility: style.contentVisibility,
      contain: style.contain,
      containIntrinsicSize: style.containIntrinsicSize,
      containIntrinsicHeight: style.getPropertyValue("contain-intrinsic-height"),
    };
  };
  return { first: read(first), far: read(far) };
}

async function beginEdit(rowId: string): Promise<Record<string, unknown>> {
  const row = scrollElement().querySelector<HTMLElement>(`[data-row-id="${rowId}"]`);
  if (row === null) throw new Error(`no row ${rowId}`);
  const view = messages.find((message) => message.id === rowId);
  if (view === undefined) throw new Error(`no message ${rowId}`);
  scrollElement().scrollTop = Math.max(0, scrollElement().scrollTop + rectOf(row).top - 120);
  await waitForLayout();
  commit({ sections: currentSections, editing: { id: view.id, original: view.content } });
  await frames(2);
  const field = row.querySelector<HTMLTextAreaElement>("textarea.msg-edit-field");
  if (field === null) throw new Error("the editor did not open");
  field.focus();
  field.setSelectionRange(4, 4);
  return {
    rowId,
    valueLength: field.value.length,
    selectionStart: field.selectionStart,
    selectionEnd: field.selectionEnd,
    visible: rectOf(row).bottom > 0 && rectOf(row).top < scrollElement().clientHeight,
  };
}

async function checkEdit(): Promise<Record<string, unknown>> {
  const field = document.querySelector<HTMLTextAreaElement>("textarea.msg-edit-field");
  return {
    present: field !== null,
    focused: field !== null && document.activeElement === field,
    selectionStart: field?.selectionStart ?? null,
    selectionEnd: field?.selectionEnd ?? null,
    valueLength: field?.value.length ?? null,
  };
}

async function findMarker(): Promise<Record<string, unknown>> {
  const row = scrollElement().querySelector<HTMLElement>(`[data-row-id="m-${marker.index}"]`);
  if (row === null) throw new Error("the marker row is not in the DOM");
  scrollElement().focus();
  const before = rectOf(row);
  const found = (window as unknown as {
    find(
      text: string,
      caseSensitive: boolean,
      backwards: boolean,
      wrapAround: boolean,
      wholeWord: boolean,
      searchInFrames: boolean,
      showDialog: boolean,
    ): boolean;
  }).find(marker.text, false, false, true, false, false, false);
  await frames(2);
  const after = rectOf(row);
  return {
    marker: marker.text,
    found,
    selection: window.getSelection()?.toString() ?? "",
    before,
    after,
    visible: after.bottom > 0 && after.top < scrollElement().clientHeight,
  };
}

async function markerRect(): Promise<Record<string, unknown>> {
  const row = scrollElement().querySelector<HTMLElement>(`[data-row-id="m-${marker.index}"]`);
  if (row === null) throw new Error("the marker row is not in the DOM");
  return { id: row.dataset.rowId, ...rectOf(row) };
}

async function scrollToMarker(): Promise<Record<string, unknown>> {
  const row = scrollElement().querySelector<HTMLElement>(`[data-row-id="m-${marker.index}"]`);
  if (row === null) throw new Error("the marker row is not in the DOM");
  const scroller = scrollElement();
  const box = row.getBoundingClientRect();
  scroller.scrollTop = Math.max(0, scroller.scrollTop + box.top - scroller.getBoundingClientRect().top - 30);
  const layout = await waitForLayout();
  return { layout, ...snapshot() };
}

declare global {
  interface Window {
    __bench: Record<string, unknown>;
  }
}

window.__bench = {
  mount,
  append,
  prepend,
  growAbove,
  growMediaAbove,
  scrollRun,
  jump,
  replyJump,
  scrollUp,
  scrollTo,
  toolbarTarget,
  toolbarCheck,
  toolbarButtonPoint,
  toolbarState,
  pickerPoints,
  idleToolbars,
  missingToolbars,
  phoneActions,
  documentCounts,
  sweepPath,
  wheelStart,
  startFrames,
  stopFrames,
  describeFocus,
  keyboardPair,
  keyboardSpan,
  entryPoint,
  beginEdit,
  checkEdit,
  findMarker,
  markerRect,
  scrollToMarker,
  capture,
  measureSaved,
  snapshot,
  styleInfo,
  waitForLayout,
  expected: () => expectedRows(currentSections),
  markerRowId: () => `m-${marker.index}`,
  callbackInfo: () => ({ lastAction, lastEdit, lastCancel, lastReact }),
  messageCount: () => messages.length,
  visibleGroup: () => visibleGroup().dataset.rowId ?? null,
  rowIds: () => rowElements().map((row) => row.dataset.rowId ?? ""),
  marker,
  ready: true,
};
