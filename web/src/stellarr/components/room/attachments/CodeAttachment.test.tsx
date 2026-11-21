import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { tokenizeCode } from "@/lib/messages/codeLanguages";
import type { CodePart } from "@/lib/messages/parts";

import { CodeAttachment } from "./CodeAttachment";

function codePart(source: string): CodePart {
  return {
    kind: "code",
    lang: "example.ts",
    source,
    language: "typescript",
    tokens: tokenizeCode(source, "typescript"),
  };
}

describe("CodeAttachment", () => {
  it("recolours the block for the reader without changing the source", async () => {
    const user = userEvent.setup();
    const source = "const value = 1\ndef greet():\n  print('hej')";
    const onCopy = vi.fn();
    render(<CodeAttachment part={codePart(source)} onCopy={onCopy} />);

    const picker = screen.getByRole("combobox", { name: "Podświetl jako" });
    expect(picker).toHaveTextContent("TypeScript");
    expect(screen.getByText("const", { selector: ".kw" })).toBeInTheDocument();
    expect(screen.queryByText("def", { selector: ".kw" })).toBeNull();

    await user.click(picker);
    await user.click(screen.getByRole("option", { name: "Python" }));

    expect(picker).toHaveTextContent("Python");
    expect(screen.getByText("def", { selector: ".kw" })).toBeInTheDocument();
    expect(screen.getByLabelText("Kod: Python")).toHaveTextContent("const value = 1");

    await user.click(screen.getByRole("button", { name: "Kopiuj kod" }));
    expect(onCopy).toHaveBeenCalledWith(expect.objectContaining({ source, language: "python" }));
  });

  it("says the code was copied, and offers a retry when copying fails", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("denied"));
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    render(<CodeAttachment part={codePart("const a = 1")} />);

    await user.click(screen.getByRole("button", { name: "Kopiuj kod" }));
    expect(writeText).toHaveBeenCalledWith("const a = 1");
    const copied = screen.getByRole("button", { name: "Skopiowano kod" });
    expect(copied).toHaveTextContent("Skopiowano");

    await user.click(copied);
    expect(screen.getByRole("button", { name: "Ponów kopiowanie kodu" })).not.toHaveTextContent(
      "Skopiowano",
    );
  });

  it("collapses a long listing and opens it from the bottom of the block", async () => {
    const user = userEvent.setup();
    const source = Array.from({ length: 20 }, (_, index) => `const line${index} = ${index};`).join("\n");
    render(<CodeAttachment part={codePart(source)} />);

    const pre = screen.getByLabelText("Kod: TypeScript");
    expect(pre).toHaveClass("compact");
    expect(pre).toHaveAttribute("tabindex", "0");
    expect(screen.getByText("20 linii")).toBeInTheDocument();

    const more = screen.getByRole("button", { name: "Pokaż całość" });
    expect(more).toHaveAttribute("aria-expanded", "false");
    // After the code, where the listing breaks off.
    expect(pre.compareDocumentPosition(more) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    await user.click(more);
    expect(pre).toHaveClass("expanded");
    expect(screen.getByRole("button", { name: "Zwiń" })).toHaveAttribute("aria-expanded", "true");
  });

  it("leaves a short listing open, with nothing to expand", () => {
    render(<CodeAttachment part={codePart("a\nb\nc")} />);

    expect(screen.getByText("3 linie")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Pokaż całość" })).toBeNull();
  });
});
