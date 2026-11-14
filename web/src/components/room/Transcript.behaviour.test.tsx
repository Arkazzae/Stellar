import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { fakeTranscriptLayout } from "@/test/transcriptLayout";

import type * as MessageGroupModule from "./MessageGroup";
import type { MessageGroupProps, MessageGroupVM } from "./MessageGroup";
import { Transcript, type TranscriptSectionVM } from "./Transcript";

/** The props each block was last rendered with, by block id. */
const rendered = vi.hoisted(() => new Map<string, MessageGroupProps>());

// The real block renders; this only notes what the transcript handed it, for
// the props that have no visible effect on an empty message.
vi.mock("./MessageGroup", async (importOriginal) => {
  const actual = await importOriginal<typeof MessageGroupModule>();
  return {
    ...actual,
    MessageGroup: (props: MessageGroupProps) => {
      rendered.set(props.id, props);
      return createElement(actual.MessageGroup, props);
    },
  };
});

function group(id: string, ...bodyIds: string[]): MessageGroupVM {
  return {
    id,
    authorId: "user-1",
    authorName: "Maja",
    timestamp: "2026-08-14T10:00:00Z",
    bodies: (bodyIds.length > 0 ? bodyIds : [id]).map((bodyId) => ({
      id: bodyId,
      parts: [],
    })),
  };
}

function section(...groups: MessageGroupVM[]): TranscriptSectionVM[] {
  return [{ day: "Dzisiaj", groups }];
}

/** `m1` … `mN` as blocks: with the day separator, N + 1 rows of 100 px. */
function blocks(count: number): MessageGroupVM[] {
  return Array.from({ length: count }, (_, index) => group(`m${index + 1}`));
}

/** Message blocks ask before deleting, so they render under the app's confirm dialog. */
const show = (ui: ReactElement) => render(ui, { wrapper: ConfirmProvider });

const rowIds = () =>
  [...screen.getByRole("log").querySelectorAll<HTMLElement>("[data-row-id]")].map(
    (row) => row.dataset.rowId,
  );

function propsOf(id: string): MessageGroupProps {
  const props = rendered.get(id);
  if (props === undefined) throw new Error(`no block ${id}`);
  return props;
}

beforeEach(() => rendered.clear());

afterEach(() => vi.useRealTimers());

describe("Transcript rows", () => {
  it("draws the unread line above the first unread block only", () => {
    show(
      <Transcript
        sections={section(group("a"), group("b"), group("c"))}
        label="Wiadomości"
        firstUnreadGroupId="b"
      />,
    );

    expect(rowIds()).toEqual(["day-Dzisiaj", "a", "unread-b", "b", "c"]);
    expect(screen.getByText("Nowe wiadomości")).toBeInTheDocument();
  });

  it("writes the day heading as its own row", () => {
    show(<Transcript sections={section(group("a"))} label="Wiadomości" />);

    expect(screen.getByText("Dzisiaj")).toHaveClass("day-sep");
  });

  it("renders the intro inside the list, above the first day", () => {
    show(
      <Transcript
        sections={section(group("a"))}
        label="Wiadomości"
        intro={<p>To początek kanału.</p>}
      />,
    );

    const log = screen.getByRole("log");
    const intro = screen.getByText("To początek kanału.");
    expect(log).toContainElement(intro);
    expect(
      intro.compareDocumentPosition(screen.getByText("Dzisiaj")) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("has nothing above the rows when there is no intro", () => {
    show(<Transcript sections={section(group("a"))} label="Wiadomości" />);

    expect(screen.getByRole("log").firstElementChild).toHaveClass("transcript-inner");
  });

  it("raises the profile of the author of the block it was clicked on", async () => {
    const user = userEvent.setup();
    const onOpenAuthor = vi.fn();
    show(
      <Transcript sections={section(group("a"))} label="Wiadomości" onOpenAuthor={onOpenAuthor} />,
    );

    await user.click(screen.getByRole("button", { name: "Profil: Maja" }));

    expect(onOpenAuthor).toHaveBeenCalledWith("user-1", expect.any(HTMLElement));
  });

  it("leaves authors inert when nothing can open a profile", () => {
    show(<Transcript sections={section(group("a"))} label="Wiadomości" />);

    expect(screen.queryByRole("button", { name: "Profil: Maja" })).toBeNull();
  });

  it("is a labelled log that announces what is added", () => {
    show(<Transcript sections={section(group("a"))} label="Wiadomości w #ogólny" />);

    const log = screen.getByRole("log", { name: "Wiadomości w #ogólny" });
    expect(log).toHaveAttribute("aria-live", "polite");
    expect(log).toHaveAttribute("aria-relevant", "additions");
    expect(log).toHaveAttribute("aria-busy", "false");
  });
});

describe("Transcript older history", () => {
  function reader(props: { hasOlder?: boolean; loadingOlder?: boolean; onLoadOlder?: () => void }) {
    const layout = fakeTranscriptLayout();
    const view = show(<Transcript sections={section(...blocks(20))} label="Wiadomości" {...props} />);
    return { layout, ...view };
  }

  it("asks for the next page as the reader nears the top", () => {
    const onLoadOlder = vi.fn();
    const { layout } = reader({ hasOlder: true, onLoadOlder });
    // Opened at the end, 1 500 px down: nowhere near the top yet.
    expect(onLoadOlder).not.toHaveBeenCalled();

    layout.scrollTo(1_000);
    expect(onLoadOlder).not.toHaveBeenCalled();
    layout.scrollTo(800);
    expect(onLoadOlder).toHaveBeenCalledOnce();

    // Still near the top, with the same page outstanding.
    layout.scrollTo(400);
    expect(onLoadOlder).toHaveBeenCalledOnce();
  });

  it("does not ask when there is nothing older", () => {
    const onLoadOlder = vi.fn();
    const { layout } = reader({ onLoadOlder });

    layout.scrollTo(0);

    expect(onLoadOlder).not.toHaveBeenCalled();
  });

  it("does not ask again while a page is already loading", () => {
    const onLoadOlder = vi.fn();
    const { layout } = reader({ hasOlder: true, loadingOlder: true, onLoadOlder });

    layout.scrollTo(0);

    expect(onLoadOlder).not.toHaveBeenCalled();
  });

  it("does not ask when nobody listens", () => {
    const { layout } = reader({ hasOlder: true });

    expect(() => layout.scrollTo(0)).not.toThrow();
  });

  it("asks straight away when the first page does not fill the pane", () => {
    fakeTranscriptLayout();
    const onLoadOlder = vi.fn();

    show(
      <Transcript sections={section(...blocks(3))} label="Wiadomości" hasOlder onLoadOlder={onLoadOlder} />,
    );

    expect(onLoadOlder).toHaveBeenCalledOnce();
  });

  it("asks again when a page lands and the top is still in reach", () => {
    fakeTranscriptLayout();
    const onLoadOlder = vi.fn();
    const props = { label: "Wiadomości", hasOlder: true, onLoadOlder };
    const { rerender } = show(<Transcript sections={section(...blocks(3))} {...props} />);
    rerender(<Transcript sections={section(...blocks(3))} {...props} loadingOlder />);

    // A short page: two older blocks, and the pane is still not full.
    rerender(<Transcript sections={section(group("o1"), group("o2"), ...blocks(3))} {...props} />);

    expect(onLoadOlder).toHaveBeenCalledTimes(2);
  });

  it("does not retry a page that failed until the reader comes back to the top", () => {
    const layout = fakeTranscriptLayout();
    const onLoadOlder = vi.fn();
    const props = { sections: section(...blocks(20)), label: "Wiadomości", hasOlder: true, onLoadOlder };
    const { rerender } = show(<Transcript {...props} />);
    layout.scrollTo(300);
    expect(onLoadOlder).toHaveBeenCalledOnce();

    // The request failed: loading ends and the rows are the same.
    rerender(<Transcript {...props} loadingOlder />);
    rerender(<Transcript {...props} />);
    layout.scrollTo(200);
    expect(onLoadOlder).toHaveBeenCalledOnce();

    // Another trip to the top tries again.
    layout.scrollTo(1_200);
    layout.scrollTo(500);
    expect(onLoadOlder).toHaveBeenCalledTimes(2);
  });

  it("does not ask from a transcript that is hidden", () => {
    const layout = fakeTranscriptLayout({ viewport: 0 });
    const onLoadOlder = vi.fn();

    show(
      <Transcript sections={section(...blocks(3))} label="Wiadomości" hasOlder onLoadOlder={onLoadOlder} />,
    );
    layout.scrollTo(0);

    expect(onLoadOlder).not.toHaveBeenCalled();
  });
});

describe("Transcript jumping to a message", () => {
  function mount() {
    const layout = fakeTranscriptLayout();
    let jumpTo: ((messageId: string) => void) | undefined;
    show(
      <Transcript
        sections={section(...blocks(20), group("b", "b1", "b2"))}
        label="Wiadomości"
        onJumpReady={(jump) => {
          jumpTo = jump;
        }}
      />,
    );
    if (jumpTo === undefined) throw new Error("the jump was not handed out");
    const scrollTo = vi.spyOn(layout.scroller(), "scrollTo");
    return { layout, jumpTo, scrollTo };
  }

  it("scrolls smoothly to the block that holds the message, centred, and flashes it", () => {
    vi.useFakeTimers();
    const { jumpTo, scrollTo } = mount();

    act(() => jumpTo("m5"));

    // m5 is the sixth row, 500..600 down the content; centred in a 600 px
    // pane, its top sits 250 px below the viewport's.
    expect(scrollTo).toHaveBeenCalledWith({ top: 250, behavior: "smooth" });
    expect(propsOf("m5").highlight).toBe(true);
    expect(propsOf("m4").highlight).toBe(false);

    act(() => vi.advanceTimersByTime(1_200));
    expect(propsOf("m5").highlight).toBe(false);
  });

  it("finds a later message of a run by the block that holds it", () => {
    const { layout, jumpTo, scrollTo } = mount();
    layout.scrollTo(0);

    act(() => jumpTo("b2"));

    expect(scrollTo).toHaveBeenCalledWith({ top: layout.end(), behavior: "smooth" });
    expect(propsOf("b").highlight).toBe(true);
  });

  it("does nothing for a message that has not been loaded", () => {
    const { jumpTo, scrollTo } = mount();

    act(() => jumpTo("older"));

    expect(scrollTo).not.toHaveBeenCalled();
    expect(propsOf("m1").highlight).toBe(false);
  });

  it("keeps aiming at the block when a row above it changes height on the way", () => {
    const { layout, jumpTo, scrollTo } = mount();

    act(() => jumpTo("m5"));
    // A picture above m5 finished loading while the scroll was travelling,
    // and m5 is now 200 px further down.
    layout.set({ heights: { m2: 300 } });
    layout.resized();

    expect(scrollTo).toHaveBeenLastCalledWith({ top: 450, behavior: "smooth" });
  });

  it("lets the reader take the scroller back from a jump still on its way", () => {
    const { layout, jumpTo, scrollTo } = mount();
    act(() => jumpTo("m5"));
    // Partway there, the reader turns the wheel.
    layout.scrollTo(900);
    act(() => fireEvent.wheel(layout.scroller()));
    const reading = layout.topOf("m9");

    layout.set({ heights: { m2: 300 } });
    layout.resized();

    // Not pulled on towards m5: the row being read simply stays put.
    expect(scrollTo).toHaveBeenCalledTimes(1);
    expect(layout.topOf("m9")).toBe(reading);
  });

  it("is the same function the reply line uses, and is handed out once", async () => {
    const user = userEvent.setup();
    const layout = fakeTranscriptLayout();
    const onJumpReady = vi.fn();
    const answer = group("answer");
    answer.replyTo = { id: "m1", authorId: "u2", authorName: "Nils", preview: "Cześć" };
    const { rerender } = show(
      <Transcript sections={section(...blocks(20), answer)} label="Wiadomości" onJumpReady={onJumpReady} />,
    );
    const scrollTo = vi.spyOn(layout.scroller(), "scrollTo");

    await user.click(screen.getByRole("button", { name: /Cześć/ }));
    expect(scrollTo).toHaveBeenLastCalledWith({ top: 0, behavior: "smooth" });

    rerender(
      <Transcript
        sections={section(...blocks(20), answer, group("later"))}
        label="Wiadomości"
        onJumpReady={onJumpReady}
      />,
    );
    expect(onJumpReady).toHaveBeenCalledTimes(1);
    expect(onJumpReady.mock.calls[0]?.[0]).toBe(propsOf("answer").onOpenReply);
  });

  it("finds a message that arrived after the jump was handed out", () => {
    const layout = fakeTranscriptLayout();
    let jumpTo: ((messageId: string) => void) | undefined;
    const onJumpReady = (jump: (messageId: string) => void) => {
      jumpTo = jump;
    };
    const { rerender } = show(
      <Transcript sections={section(...blocks(20))} label="Wiadomości" onJumpReady={onJumpReady} />,
    );
    rerender(
      <Transcript sections={section(...blocks(20), group("b"))} label="Wiadomości" onJumpReady={onJumpReady} />,
    );
    layout.scrollTo(0);
    const scrollTo = vi.spyOn(layout.scroller(), "scrollTo");

    act(() => jumpTo?.("b"));

    expect(scrollTo).toHaveBeenCalledWith({ top: layout.end(), behavior: "smooth" });
  });

  it("leaves reply lines out inside a thread", () => {
    const answer = group("answer");
    answer.replyTo = { id: "root", authorId: "u2", authorName: "Nils", preview: "Cześć" };
    show(<Transcript sections={section(answer)} label="Wątek" hideReplyReferences />);

    expect(screen.queryByRole("button", { name: /Cześć/ })).toBeNull();
    expect(propsOf("answer").onOpenReply).toBeUndefined();
  });
});

describe("Transcript arrivals", () => {
  it("marks a block fresh only while it is new, and never on the first render", () => {
    vi.useFakeTimers();
    const { rerender } = show(<Transcript sections={section(group("a"))} label="Wiadomości" />);
    expect(propsOf("a").fresh).toBe(false);

    rerender(<Transcript sections={section(group("a"), group("b"))} label="Wiadomości" />);
    expect(propsOf("b").fresh).toBe(true);
    expect(propsOf("a").fresh).toBe(false);

    act(() => vi.advanceTimersByTime(400));
    expect(propsOf("b").fresh).toBe(false);
  });

  it("does not treat the first page of an empty transcript as something missed", () => {
    fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={[]} label="Wiadomości" />);

    rerender(<Transcript sections={section(...blocks(20))} label="Wiadomości" />);

    expect(screen.queryByRole("button", { name: /Nowe wiadomości/ })).toBeNull();
  });

  it("goes back to the end and drops the pill's name when it is pressed", async () => {
    const user = userEvent.setup();
    const layout = fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={section(...blocks(20))} label="Wiadomości" />);
    layout.scrollTo(200);
    rerender(<Transcript sections={section(...blocks(21))} label="Wiadomości" />);
    const scrollTo = vi.spyOn(layout.scroller(), "scrollTo");

    await user.click(screen.getByRole("button", { name: /Nowe wiadomości/ }));

    expect(scrollTo).toHaveBeenCalledWith({ top: 1_600, behavior: "smooth" });
    expect(screen.getByRole("button", { name: "Przejdź do najnowszych wiadomości" })).toBeVisible();
  });
});
