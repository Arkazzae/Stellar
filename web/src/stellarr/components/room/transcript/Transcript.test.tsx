import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode, type ReactElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { fakeTranscriptLayout } from "@/test/transcriptLayout";

import type { MessageGroupVM } from "./MessageGroup";
import { Transcript, type TranscriptSectionVM } from "./Transcript";

function group(id: string, ...bodyIds: string[]): MessageGroupVM {
  return {
    id,
    authorId: "user-1",
    authorName: "Maja",
    timestamp: "2026-08-14T10:00:00Z",
    bodies: (bodyIds.length > 0 ? bodyIds : [id]).map((bodyId) => ({ id: bodyId, parts: [] })),
  };
}

function section(...ids: string[]): TranscriptSectionVM[] {
  return [{ day: "Dzisiaj", groups: ids.map((id) => group(id)) }];
}

/** `m1` … `mN`: with the day separator, N + 1 rows of 100 px. */
function messages(count: number, prefix = "m"): string[] {
  return Array.from({ length: count }, (_, index) => `${prefix}${index + 1}`);
}

/** Message blocks ask before deleting, so they render under the app's confirm dialog. */
const show = (ui: ReactElement) => render(ui, { wrapper: ConfirmProvider });

/** The way back down, in either of its two shapes. */
const catchUpButton = () =>
  screen.queryByRole("button", { name: /^(Przejdź do najnowszych wiadomości|Nowe wiadomości)$/ });

describe("Transcript opening", () => {
  it("keeps an empty busy log while the first page loads, then opens at its end before painting", () => {
    const layout = fakeTranscriptLayout();
    const { rerender } = show(<Transcript loading sections={section(...messages(10))} label="Wiadomości" />,
    );

    expect(screen.getByRole("log")).toHaveAttribute("aria-busy", "true");
    expect(screen.queryByText("Dzisiaj")).toBeNull();

    rerender(<Transcript sections={section(...messages(10))} label="Wiadomości" />);

    // 11 rows of 100 px in a 600 px pane, and nothing waited for.
    expect(layout.scroller().scrollTop).toBe(500);
    expect(screen.getByRole("log")).toHaveAttribute("aria-busy", "false");
    expect(catchUpButton()).toBeNull();
  });

  it("is already at its end when it is too short to scroll", () => {
    const layout = fakeTranscriptLayout();
    show(<Transcript sections={section("m1", "m2")} label="Wiadomości" />);

    expect(layout.scroller().scrollTop).toBe(0);
    // The rows sit at the bottom of the pane, under the free space.
    expect(layout.topOf("m2")).toBe(500);
    expect(catchUpButton()).toBeNull();
  });

  it("opens at its end under Strict Mode, which runs every effect twice", () => {
    const layout = fakeTranscriptLayout();
    show(
      <StrictMode>
        <Transcript sections={section(...messages(10))} label="Wiadomości" />
      </StrictMode>,
    );

    expect(layout.scroller().scrollTop).toBe(500);
    expect(catchUpButton()).toBeNull();
  });

  it("asks for one older page, not two, when a short first page opens under Strict Mode", () => {
    fakeTranscriptLayout();
    const onLoadOlder = vi.fn();
    show(
      <StrictMode>
        <Transcript sections={section("m1", "m2")} label="Wiadomości" hasOlder onLoadOlder={onLoadOlder} />
      </StrictMode>,
    );

    expect(onLoadOlder).toHaveBeenCalledOnce();
  });

  it("hides repeated reply references inside a thread without changing its data", () => {
    fakeTranscriptLayout();
    const answer = group("answer-1");
    answer.replyTo = {
      id: "root-1",
      authorId: "user-2",
      authorName: "Nils",
      preview: "Wiadomość rozpoczynająca wątek",
    };
    show(<Transcript sections={[{ day: "Dzisiaj", groups: [answer] }]} label="Wątek" hideReplyReferences />,
    );

    expect(screen.queryByText("Wiadomość rozpoczynająca wątek")).toBeNull();
    expect(answer.replyTo?.id).toBe("root-1");
  });
});

describe("Transcript keeps the reader's place", () => {
  it("holds the row being read when history and a live message arrive together", () => {
    const layout = fakeTranscriptLayout();
    const ids = messages(10);
    const { rerender } = show(<Transcript sections={section(...ids)} label="Wiadomości" />);
    // m1 straddles the top edge, m2 starts 50 px into the pane.
    layout.scrollTo(150);
    expect(layout.topOf("m2")).toBe(50);

    // Three older messages landed above while m11 arrived below. Counting only
    // the rows that were prepended would be thrown off by the second change.
    rerender(
      <Transcript sections={section("a1", "a2", "a3", ...ids, "m11")} label="Wiadomości" />,
    );

    expect(layout.topOf("m1")).toBe(-50);
    expect(layout.topOf("m2")).toBe(50);
    expect(layout.scroller().scrollTop).toBe(450);
  });

  it("holds the next row when the page merged the reader's block into an older one", () => {
    const layout = fakeTranscriptLayout();
    const ids = messages(10);
    const { rerender } = show(<Transcript sections={section(...ids)} label="Wiadomości" />);
    layout.scrollTo(150);

    // m1's message now belongs to a block that starts with an older one, under
    // that one's id; m1's own row is gone.
    layout.set({ heights: { a1: 200 } });
    rerender(
      <Transcript
        sections={[{ day: "Dzisiaj", groups: [group("a0"), group("a1", "a1", "m1"), ...ids.slice(1).map((id) => group(id))] }]}
        label="Wiadomości"
      />,
    );

    expect(layout.topOf("m2")).toBe(50);
  });

  it("keeps the distance from the end when nothing near the reader survived", () => {
    const layout = fakeTranscriptLayout();
    // One long run of one author: a single block, taller than the pane.
    layout.set({ heights: { run: 2_000 } });
    const { rerender } = show(<Transcript sections={[{ day: "Dzisiaj", groups: [group("run", "r1", "r2")] }]} label="Wiadomości" />,
    );
    layout.scrollTo(300);

    // The page before it continues the run, so the block takes the older head's id.
    layout.set({ heights: { older: 3_000 } });
    rerender(
      <Transcript
        sections={[{ day: "Dzisiaj", groups: [group("older", "r0", "r1", "r2")] }]}
        label="Wiadomości"
      />,
    );

    expect(layout.scroller().scrollTop).toBe(1_300);
  });

  it("does not move what the reader sees when a row above them grows", () => {
    const layout = fakeTranscriptLayout();
    show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);
    layout.scrollTo(1_000);
    expect(layout.topOf("m10")).toBe(0);

    // An image in m3, far above the viewport, finished loading.
    layout.set({ heights: { m3: 400 } });
    layout.resized();

    expect(layout.topOf("m10")).toBe(0);
    expect(layout.scroller().scrollTop).toBe(1_300);
  });

  it("follows content that grows under a reader exactly at the end", () => {
    const layout = fakeTranscriptLayout();
    show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);
    expect(layout.scroller().scrollTop).toBe(1_500);

    layout.set({ heights: { m20: 300 } });
    layout.resized();

    expect(layout.scroller().scrollTop).toBe(1_700);
    expect(catchUpButton()).toBeNull();
  });

  it("leaves a reader 20 px above the end where they are when content grows below", () => {
    const layout = fakeTranscriptLayout();
    show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);
    layout.scrollTo(1_480);

    layout.set({ heights: { m20: 300 } });
    layout.resized();

    expect(layout.scroller().scrollTop).toBe(1_480);
    // 220 px from the end is reading, so the way back down appears.
    expect(catchUpButton()).toBeVisible();
  });

  it("keeps the end in view when the pane gets shorter under a reader at the end", () => {
    const layout = fakeTranscriptLayout();
    show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);

    // The composer grew by a line.
    layout.set({ viewport: 500 });
    layout.resized();

    expect(layout.scroller().scrollTop).toBe(1_600);
  });

  it("opens another conversation at its end when every message is replaced", () => {
    const layout = fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={section(...messages(20))} label="Wiadomości z Ada" />,
    );
    layout.scrollTo(200);
    expect(catchUpButton()).toBeVisible();

    rerender(<Transcript sections={section(...messages(20, "x"))} label="Wiadomości z Ola" />);

    expect(layout.scroller().scrollTop).toBe(1_500);
    expect(catchUpButton()).toBeNull();
  });

  it("opens at the end when the first page lands in an empty transcript", () => {
    const layout = fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={[]} label="Wiadomości" />);

    rerender(<Transcript sections={section(...messages(20))} label="Wiadomości" />);

    expect(layout.scroller().scrollTop).toBe(1_500);
    expect(catchUpButton()).toBeNull();
  });

  it("does not take the place of a hidden transcript as the reader's", () => {
    const layout = fakeTranscriptLayout();
    show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);
    layout.scrollTo(1_000);

    // Behind a call: no box, everything measures zero, and the offset is lost.
    layout.set({ viewport: 0 });
    layout.resized();
    layout.set({ viewport: 600 });
    layout.resized();

    // Back in view, at the row that was being read.
    expect(layout.topOf("m10")).toBe(0);
    expect(catchUpButton()).toBeVisible();
  });
});

describe("Transcript following arrivals", () => {
  it("follows an arrival before it is painted, and not its server reconciliation", () => {
    const layout = fakeTranscriptLayout();
    const ids = messages(10);
    const { rerender } = show(<Transcript
        sections={[{ day: "Dzisiaj", groups: [...ids.map((id) => group(id)), group("g", "g1")] }]}
        label="Wiadomości"
      />,
    );

    // A second message from the same author grows the last block in place.
    layout.set({ heights: { g: 124 } });
    rerender(
      <Transcript
        sections={[{ day: "Dzisiaj", groups: [...ids.map((id) => group(id)), group("g", "g1", "local-2")] }]}
        label="Wiadomości"
      />,
    );
    expect(layout.scroller().scrollTop).toBe(layout.end());

    // The reader moves a little up, still close enough to count as at the end.
    layout.scrollTo(layout.end() - 30);
    const reading = layout.scroller().scrollTop;

    // The server's id replaces the optimistic one and the block re-measures.
    // That is not a second arrival, so the reader is not pulled down.
    layout.set({ heights: { g: 140 } });
    rerender(
      <Transcript
        sections={[{ day: "Dzisiaj", groups: [...ids.map((id) => group(id)), group("g", "g1", "server-2")] }]}
        label="Wiadomości"
      />,
    );
    expect(layout.scroller().scrollTop).toBe(reading);
  });

  it("does not read a followed row that grows as the reader leaving", () => {
    const layout = fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);
    rerender(<Transcript sections={section(...messages(21))} label="Wiadomości" />);

    // The new message's picture loads. The scroll event for the follow is
    // delivered after that and before the resize is reported.
    layout.set({ heights: { m21: 400 } });
    act(() => fireEvent.scroll(layout.scroller()));
    expect(catchUpButton()).toBeNull();
    layout.resized();

    expect(layout.scroller().scrollTop).toBe(layout.end());
    expect(catchUpButton()).toBeNull();
  });

  it("follows an arrival for a reader within a few lines of the end", () => {
    const layout = fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);
    layout.scrollTo(1_470);

    rerender(<Transcript sections={section(...messages(21))} label="Wiadomości" />);

    expect(layout.scroller().scrollTop).toBe(1_600);
  });
});

describe("Transcript own send", () => {
  function readerScrolledUp() {
    const layout = fakeTranscriptLayout();
    const view = show(<Transcript sections={section(...messages(20))} label="Wiadomości" followOwnSend={0} />,
    );
    layout.scrollTo(120);
    return { ...view, layout };
  }

  it("goes to your own message even when you were reading further up", () => {
    const { rerender, layout } = readerScrolledUp();

    rerender(
      <Transcript sections={section(...messages(21))} label="Wiadomości" followOwnSend={1} />,
    );

    // 22 rows in a 600 px pane: the send is an action, so it commands the view.
    expect(layout.scroller().scrollTop).toBe(1_600);
    expect(catchUpButton()).toBeNull();
  });

  it("leaves a reader alone when the arrival is somebody else's", () => {
    const { rerender, layout } = readerScrolledUp();

    rerender(
      <Transcript sections={section(...messages(21))} label="Wiadomości" followOwnSend={0} />,
    );

    // Being yanked mid-read is worse than missing a line, so nothing moves.
    expect(layout.scroller().scrollTop).toBe(120);
  });
});

describe("Transcript catch-up button", () => {
  function arrive(rerender: (ui: ReactElement) => void, ...ids: string[]) {
    rerender(<Transcript sections={section(...ids)} label="Wiadomości" />);
  }

  it("stays away while the reader is on the newest message", () => {
    fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);

    arrive(rerender, ...messages(21));

    // The list follows the end for a reader who is on it; nothing to go back to.
    expect(catchUpButton()).toBeNull();
  });

  it("appears as soon as the end is off screen, before anything arrives", async () => {
    const user = userEvent.setup();
    const layout = fakeTranscriptLayout();
    show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);
    const scrollTo = vi.spyOn(layout.scroller(), "scrollTo");

    layout.scrollTo(1_000);

    // Nothing has happened down there yet, so it is an arrow and nothing else.
    const jump = screen.getByRole("button", { name: "Przejdź do najnowszych wiadomości" });
    expect(jump).toHaveTextContent("");

    await user.click(jump);
    expect(scrollTo).toHaveBeenCalledWith({ top: 1_500, behavior: "smooth" });
  });

  it("counts a reader within a few lines of the end as being there", () => {
    const layout = fakeTranscriptLayout();
    show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);

    layout.scrollTo(1_460);
    expect(catchUpButton()).toBeNull();

    layout.scrollTo(1_440);
    expect(catchUpButton()).toBeVisible();
  });

  it("names itself once a message lands while the reader is away", () => {
    const layout = fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);

    layout.scrollTo(200);
    expect(screen.queryByRole("button", { name: /Nowe wiadomości/ })).toBeNull();

    arrive(rerender, ...messages(21));
    expect(screen.getByRole("button", { name: /Nowe wiadomości/ })).toBeVisible();
  });

  it("leaves of its own accord when the reader scrolls back down", () => {
    const layout = fakeTranscriptLayout();
    const { rerender } = show(<Transcript sections={section(...messages(20))} label="Wiadomości" />);

    layout.scrollTo(200);
    arrive(rerender, ...messages(21));
    expect(screen.getByRole("button", { name: /Nowe wiadomości/ })).toBeVisible();

    layout.scrollTo(layout.end());
    expect(catchUpButton()).toBeNull();
  });
});

describe("Transcript quick reactions", () => {
  it("offers them on its messages only while you may add reactions", () => {
    localStorage.clear();
    fakeTranscriptLayout();
    const sections = section("m1");
    const quick = () => screen.queryAllByRole("button", { name: /^Zareaguj: / });
    const { rerender } = show(<Transcript sections={sections} label="Wiadomości" onReact={vi.fn()} />);
    fireEvent.mouseOver(document.querySelector(".msg-entry") as HTMLElement);
    expect(quick()).toHaveLength(3);

    // The same rows: only the permission changed.
    rerender(
      <Transcript sections={sections} label="Wiadomości" onReact={vi.fn()} canAddReactions={false} />,
    );
    expect(quick()).toEqual([]);
  });
});
