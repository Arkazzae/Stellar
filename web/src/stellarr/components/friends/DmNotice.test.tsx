import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { TranscriptNotice } from "@/components/room/transcript/MessageGroup";

import { DmNotice, type DmNoticeContext } from "./DmNotice";

const SELF = "u-me";
const PEER = "u-maja";

function context(overrides: Partial<DmNoticeContext> = {}): DmNoticeContext {
  return {
    selfId: SELF,
    peerName: "Maja Nowak",
    liveRequestId: "r-1",
    declinedIds: new Set(),
    dismissedRequests: new Set(),
    encryptionOn: false,
    canEnterPin: true,
    unsupportedReason: null,
    busy: false,
    onCancelRequest: vi.fn(),
    onConfirmRequest: vi.fn(),
    onDismissRequest: vi.fn(),
    onCompareCode: vi.fn(),
    onEnterPin: vi.fn(),
    ...overrides,
  };
}

const request = (actorId: string): TranscriptNotice => ({
  kind: "line",
  line: { kind: "encryption_requested", actorId },
  actorName: actorId === SELF ? "Owner" : "Maja Nowak",
});

describe("DmNotice", () => {
  it("shows the other person's waiting request as a card that confirms, or is put aside for now", async () => {
    const user = userEvent.setup();
    const ctx = context();
    render(<DmNotice notice={request(PEER)} lineId="r-1" authorId={PEER} context={ctx} />);

    await user.click(screen.getByRole("button", { name: "Potwierdź" }));
    await user.click(screen.getByRole("button", { name: "Nie teraz" }));

    expect(ctx.onConfirmRequest).toHaveBeenCalledOnce();
    expect(ctx.onDismissRequest).toHaveBeenCalledWith("r-1");
  });

  it("keeps a request put aside as a line that can still be confirmed", async () => {
    const user = userEvent.setup();
    const ctx = context({ dismissedRequests: new Set(["r-1"]) });
    render(<DmNotice notice={request(PEER)} lineId="r-1" authorId={PEER} context={ctx} />);

    expect(screen.queryByRole("button", { name: "Nie teraz" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Potwierdź" }));
    expect(ctx.onConfirmRequest).toHaveBeenCalledOnce();
  });

  it("lets the requester withdraw a waiting request, and offers nothing on an old one", async () => {
    const user = userEvent.setup();
    const ctx = context();
    const { rerender } = render(<DmNotice notice={request(SELF)} lineId="r-1" authorId={SELF} context={ctx} />);
    await user.click(screen.getByRole("button", { name: "Anuluj prośbę" }));
    expect(ctx.onCancelRequest).toHaveBeenCalledOnce();

    rerender(<DmNotice notice={request(SELF)} lineId="r-0" authorId={SELF} context={ctx} />);
    expect(screen.getByRole("note")).toHaveTextContent("Prośba o szyfrowanie wysłana.");
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("offers the PIN for history the account key opens, and nothing for history no key here opens", async () => {
    const user = userEvent.setup();
    const ctx = context();
    const { rerender } = render(
      <DmNotice notice={{ kind: "sealed", reason: "pin", count: 3 }} lineId="s-1" authorId={PEER} context={ctx} />,
    );
    expect(screen.getByRole("note")).toHaveTextContent("3 zaszyfrowane wiadomości");
    await user.click(screen.getByRole("button", { name: "Wpisz PIN" }));
    expect(ctx.onEnterPin).toHaveBeenCalledOnce();

    rerender(<DmNotice notice={{ kind: "sealed", reason: "lost", count: 5 }} lineId="s-1" authorId={PEER} context={ctx} />);
    expect(screen.getByRole("note")).toHaveTextContent("5 zaszyfrowanych wiadomości");
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("offers to compare the code after the other person's keys changed", async () => {
    const user = userEvent.setup();
    const ctx = context({ encryptionOn: true });
    render(
      <DmNotice
        notice={{ kind: "line", line: { kind: "keys_reset", actorId: PEER }, actorName: "Maja Nowak" }}
        lineId="k-1"
        authorId={PEER}
        context={ctx}
      />,
    );
    expect(screen.getByRole("note")).toHaveTextContent("Kod bezpieczeństwa się zmienił: Maja Nowak ma nowe klucze.");
    await user.click(screen.getByRole("button", { name: "Porównaj kod" }));
    expect(ctx.onCompareCode).toHaveBeenCalledOnce();
  });
});
