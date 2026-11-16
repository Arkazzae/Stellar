import { render, screen } from "@testing-library/react";
import { createElement, type ReactElement } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";

import type * as MessageGroupModule from "./MessageGroup";
import type { MessageGroupProps, MessageGroupVM } from "./MessageGroup";
import { Transcript, type TranscriptSectionVM } from "./Transcript";

const renders = vi.hoisted(() => new Map<string, number>());
const lastProps = vi.hoisted(() => new Map<string, MessageGroupProps>());

vi.mock("./MessageGroup", async (importOriginal) => {
  const actual = await importOriginal<typeof MessageGroupModule>();
  return {
    ...actual,
    MessageGroup: (props: MessageGroupProps) => {
      renders.set(props.id, (renders.get(props.id) ?? 0) + 1);
      lastProps.set(props.id, props);
      return createElement(actual.MessageGroup, props);
    },
  };
});

function group(id: string, bodyId = `${id}-body`): MessageGroupVM {
  return {
    id,
    authorId: "u-1",
    authorName: "Maja",
    timestamp: "2026-08-14T10:00:00Z",
    bodies: [{ id: bodyId, parts: [] }],
  };
}

const sections = (...groups: MessageGroupVM[]): TranscriptSectionVM[] => [
  { day: "Today", groups },
];

const show = (ui: ReactElement) => render(ui, { wrapper: ConfirmProvider });

const count = (id: string) => renders.get(id) ?? 0;
const propsOf = (id: string) => {
  const props = lastProps.get(id);
  if (props === undefined) throw new Error(`no block ${id}`);
  return props;
};

beforeEach(() => {
  renders.clear();
  lastProps.clear();
});

describe("Transcript row memoisation", () => {
  it("does not redraw a block when its data is the same object", () => {
    const rows = sections(group("a"), group("b"));
    const { rerender } = show(<Transcript sections={rows} label="Messages" />);
    expect(count("a")).toBe(1);
    expect(count("b")).toBe(1);

    // The transcript itself re-renders — a label change is enough — but the
    // groups are unchanged, so their blocks are not rendered again.
    rerender(<Transcript sections={rows} label="Messages in #general" />);

    expect(count("a")).toBe(1);
    expect(count("b")).toBe(1);
  });

  it("redraws a block whose group object changed", () => {
    const { rerender } = show(
      <Transcript sections={sections(group("a"), group("b"))} label="Messages" />,
    );
    rerender(
      <Transcript sections={sections(group("a"), group("b"))} label="Messages" />,
    );

    expect(count("a")).toBe(2);
    expect(count("b")).toBe(2);
  });

  it("gives the editor to the block being edited and leaves the others alone", () => {
    const rows = sections(group("a"), group("b"));
    const { rerender } = show(<Transcript sections={rows} label="Messages" />);

    rerender(
      <Transcript
        sections={rows}
        label="Messages"
        editing={{ id: "b-body", original: "old" }}
      />,
    );

    expect(screen.getByRole("textbox")).toHaveValue("old");
    expect(count("a")).toBe(1);
    expect(count("b")).toBe(2);
  });

  it("redraws every block when the reply references are hidden", () => {
    const rows = sections(group("a"), group("b"));
    const { rerender } = show(<Transcript sections={rows} label="Messages" />);

    rerender(<Transcript sections={rows} label="Messages" hideReplyReferences />);

    expect(count("a")).toBe(2);
    expect(count("b")).toBe(2);
  });

  it("runs the newest action through a row that did not render again", () => {
    const first = vi.fn();
    const second = vi.fn();
    const rows = sections(group("a"));
    const { rerender } = show(
      <Transcript sections={rows} label="Messages" onAction={first} />,
    );

    // The provider recreates `runAction` whenever the message list changes.
    // The row must keep its stable forwarder, but that forwarder has to reach
    // the callback from this commit, not the one it was first drawn with.
    rerender(<Transcript sections={rows} label="Messages in #general" onAction={second} />);

    expect(count("a")).toBe(1);
    propsOf("a").onAction?.("a-body", "reply");
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith("a-body", "reply");
  });

  it("runs the newest save through an editor row that did not render again", () => {
    const first = vi.fn();
    const second = vi.fn();
    const rows = sections(group("a"), group("b"));
    const editing = { id: "b-body", original: "old" };
    const { rerender } = show(
      <Transcript
        sections={rows}
        label="Messages"
        editing={editing}
        onEditSave={first}
      />,
    );

    rerender(
      <Transcript
        sections={rows}
        label="Messages in #general"
        editing={editing}
        onEditSave={second}
      />,
    );

    expect(count("b")).toBe(1);
    void propsOf("b").onEditSave?.("b-body", "new");
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith("b-body", "new");
  });

  it("redraws rows when an action handler appears or disappears", () => {
    const rows = sections(group("a"));
    const action = vi.fn();
    const { rerender } = show(<Transcript sections={rows} label="Messages" />);
    expect(propsOf("a").onAction).toBeUndefined();

    rerender(<Transcript sections={rows} label="Messages" onAction={action} />);
    expect(count("a")).toBe(2);
    expect(propsOf("a").onAction).toBeTypeOf("function");

    rerender(<Transcript sections={rows} label="Messages" />);
    expect(count("a")).toBe(3);
    expect(propsOf("a").onAction).toBeUndefined();
  });
});
