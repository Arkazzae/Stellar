import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { RailThreads } from "@/components/shell/railTypes";
import type { ChannelThreadVM, TextChannelVM } from "@/components/view-models";
import {
  layOutRail,
  moveTo,
  pressAt,
  release,
  stubPointerCapture,
  translateOf,
} from "@/test/railLayout";

import { RailTextList } from "./RailTextList";

const channels: TextChannelVM[] = [
  { id: "t-1", name: "ogólny" },
  { id: "t-2", name: "plany" },
  { id: "t-3", name: "luzem" },
];

const thread = (id: string): ChannelThreadVM => ({
  id,
  channelId: "t-1",
  authorName: "Jo",
  preview: `Wątek ${id}`,
  replyCount: 1,
});

function setup({
  list = channels,
  expanded = [],
}: { list?: TextChannelVM[]; expanded?: string[] } = {}) {
  const onReorder = vi.fn();
  const onSelect = vi.fn();
  const threads: RailThreads = {
    byChannel: { "t-1": [thread("th-1"), thread("th-2")] },
    activeId: null,
    openingId: null,
    expanded: new Set(expanded),
    onToggle: vi.fn(),
    register: vi.fn(),
  };
  const view = (next: TextChannelVM[]) => (
    <RailTextList
      channels={next}
      activeChannelId={null}
      threads={threads}
      onSelect={onSelect}
      onReorder={onReorder}
    />
  );
  const { container, rerender } = render(view(list));
  return {
    onReorder,
    onSelect,
    rerender: (next: TextChannelVM[]) => rerender(view(next)),
    main: (name: string) => screen.getByRole("button", { name }),
    node: (name: string) => screen.getByRole("button", { name }).closest(".text-channel-node"),
    said: () => container.querySelector('[role="status"]')?.textContent,
  };
}

let capture: ReturnType<typeof stubPointerCapture>;

beforeEach(() => {
  layOutRail();
  capture = stubPointerCapture();
});

describe("RailTextList keyboard sorting", () => {
  it("picks a channel up with Space and still opens it with Enter", () => {
    const { main, onSelect, said } = setup();

    fireEvent.keyDown(main("ogólny"), { key: "Enter" });
    expect(onSelect).toHaveBeenCalledOnce();
    expect(said()).toBe("");

    fireEvent.keyDown(main("ogólny"), { key: " ", code: "Space" });
    expect(said()).toBe("Podniesiono kanał ogólny, pozycja 1 z 3.");
    expect(main("ogólny")).toHaveAttribute("aria-pressed", "true");
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("moves one slot per arrow, makes room for it, and drops it with Space", () => {
    const { main, node, onReorder, said } = setup();

    fireEvent.keyDown(main("ogólny"), { key: " " });
    fireEvent.keyDown(main("ogólny"), { key: "ArrowDown" });
    expect(said()).toBe("Kanał ogólny, pozycja 2 z 3.");
    expect(translateOf(node("ogólny"))).toBe("translateY(32px)");
    expect(translateOf(node("plany"))).toBe("translateY(-32px)");

    fireEvent.keyDown(main("ogólny"), { key: "ArrowDown" });
    fireEvent.keyDown(main("ogólny"), { key: "ArrowDown" });
    expect(said()).toBe("Kanał ogólny, pozycja 3 z 3.");

    fireEvent.keyDown(main("ogólny"), { key: " " });
    expect(onReorder).toHaveBeenCalledWith(["t-2", "t-3", "t-1"]);
    expect(said()).toBe("Upuszczono kanał ogólny na pozycji 3 z 3.");
    expect(translateOf(node("plany"))).toBe("");
  });

  it("keeps focus on the channel it was dropped with", () => {
    const { main, onReorder } = setup();
    // As an engine that drops focus from a node it moves would.
    onReorder.mockImplementation(() => main("ogólny").blur());

    main("ogólny").focus();
    fireEvent.keyDown(main("ogólny"), { key: " " });
    fireEvent.keyDown(main("ogólny"), { key: "ArrowDown" });
    fireEvent.keyDown(main("ogólny"), { key: " " });

    expect(onReorder).toHaveBeenCalledOnce();
    expect(main("ogólny")).toHaveFocus();
  });

  it("puts the channel back on Escape without reordering", () => {
    const { main, node, onReorder, said } = setup();

    fireEvent.keyDown(main("luzem"), { key: " " });
    fireEvent.keyDown(main("luzem"), { key: "ArrowUp" });
    fireEvent.keyDown(main("luzem"), { key: "Escape" });

    expect(onReorder).not.toHaveBeenCalled();
    expect(said()).toBe("Anulowano przestawianie kanału.");
    expect(main("luzem")).not.toHaveAttribute("aria-pressed");
    expect(translateOf(node("ogólny"))).toBe("");
  });

  it("describes the handle in the page's language", () => {
    const { main } = setup();
    const description = document.getElementById(
      main("plany").getAttribute("aria-describedby") ?? "",
    );

    expect(description).toHaveTextContent(
      "Naciśnij spację, aby podnieść kanał. Strzałkami przesuń go, spacją upuść, Escape anuluje.",
    );
    expect(main("plany")).toHaveAttribute("aria-roledescription", "kanał do przestawienia");
  });
});

describe("RailTextList pointer sorting", () => {
  it("leaves a press that does not travel 6 px a plain click", () => {
    const { main, node, onReorder, onSelect } = setup();

    pressAt(main("plany"), 148);
    moveTo(151, 153);
    expect(node("plany")).not.toHaveClass("is-dragging");
    expect(capture).not.toHaveBeenCalled();
    release(153);
    fireEvent.click(main("plany"));

    expect(onSelect).toHaveBeenCalledOnce();
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("carries the row with the pointer, shifts the others, and drops it where its centre is", () => {
    const { main, node, onReorder, onSelect, said } = setup();

    pressAt(main("ogólny"), 116);
    moveTo(124, 160);
    expect(node("ogólny")).toHaveClass("is-dragging");
    expect(capture).toHaveBeenCalledExactlyOnceWith(1);
    expect(translateOf(node("ogólny"))).toBe("translateY(44px)");
    expect(translateOf(node("plany"))).toBe("translateY(-32px)");
    expect(translateOf(node("luzem"))).toBe("");
    expect(said()).toBe("Kanał ogólny, pozycja 2 z 3.");

    release(160);
    // The click the browser sends after the release must not open the channel.
    fireEvent.click(main("ogólny"));

    expect(onReorder).toHaveBeenCalledWith(["t-2", "t-1", "t-3"]);
    expect(onSelect).not.toHaveBeenCalled();
    // Let go between slots, it eases into the one it was dropped on.
    expect(node("ogólny")).toHaveAttribute("data-settling");
  });

  it("keeps the carried row inside the list", () => {
    const { main, node, onReorder } = setup();

    pressAt(main("ogólny"), 116);
    moveTo(130, 900);
    // The list ends at 196, so the row's top can go no lower than 164.
    expect(translateOf(node("ogólny"))).toBe("translateY(64px)");

    release(900);
    fireEvent.click(main("ogólny"));
    expect(onReorder).toHaveBeenCalledWith(["t-2", "t-3", "t-1"]);
  });

  it("moves a channel's open threads with it and makes room for all of them", () => {
    const { main, node, onReorder } = setup({ expanded: ["t-1"] });

    // ogólny is 32 + 2 × 44 = 120 px tall with its threads open.
    pressAt(main("ogólny"), 116);
    moveTo(124, 200);
    expect(translateOf(node("plany"))).toBe("translateY(-120px)");

    release(200);
    fireEvent.click(main("ogólny"));
    expect(onReorder).toHaveBeenCalledWith(["t-2", "t-1", "t-3"]);
  });

  it("cancels on Escape and still eats the click of the release", () => {
    const { main, node, onReorder, onSelect, said } = setup();

    pressAt(main("ogólny"), 116);
    moveTo(124, 170);
    fireEvent.keyDown(document, { key: "Escape" });

    expect(node("ogólny")).not.toHaveClass("is-dragging");
    expect(translateOf(node("plany"))).toBe("");
    expect(said()).toBe("Anulowano przestawianie kanału.");

    release(170);
    fireEvent.click(main("ogólny"));
    expect(onReorder).not.toHaveBeenCalled();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("cancels when the browser takes the gesture over", () => {
    const { main, node, onReorder } = setup();

    pressAt(main("plany"), 148);
    moveTo(160, 190);
    fireEvent.pointerCancel(document, { pointerId: 1, isPrimary: true });

    expect(node("plany")).not.toHaveClass("is-dragging");
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("drops the drag when the carried channel is deleted under it", () => {
    const { main, node, onReorder, onSelect, rerender, said } = setup();

    pressAt(main("plany"), 148);
    moveTo(160, 190);
    rerender(channels.filter((channel) => channel.id !== "t-2"));

    expect(said()).toBe("Anulowano przestawianie kanału.");
    expect(translateOf(node("luzem"))).toBe("");

    release(190);
    fireEvent.click(main("luzem"));
    expect(onReorder).not.toHaveBeenCalled();
    expect(onSelect).not.toHaveBeenCalled();
  });
});
