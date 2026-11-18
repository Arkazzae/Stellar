import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { CodeCells } from "./CodeCells";

function Cells({ onComplete }: { onComplete(code: string): void }) {
  const [value, setValue] = useState("");
  return <CodeCells label="Kod" value={value} onChange={setValue} onComplete={onComplete} />;
}

describe("CodeCells", () => {
  it("fills every cell from a pasted code, spaces and all, and sends it once", async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    render(<Cells onComplete={onComplete} />);
    const input = screen.getByRole("textbox", { name: "Kod" });

    await user.click(input);
    await user.paste("482 913");

    expect(input).toHaveValue("482913");
    expect(onComplete).toHaveBeenCalledExactlyOnceWith("482913");

    // A seventh key changes nothing and sends nothing more.
    await user.type(input, "7");
    expect(input).toHaveValue("482913");
    expect(onComplete).toHaveBeenCalledOnce();
  });

  it("takes digits only and sends on the sixth one typed", async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    render(<Cells onComplete={onComplete} />);
    const input = screen.getByRole("textbox", { name: "Kod" });

    await user.type(input, "48a2-91");
    expect(input).toHaveValue("48291");
    expect(onComplete).not.toHaveBeenCalled();

    await user.type(input, "3");
    expect(onComplete).toHaveBeenCalledExactlyOnceWith("482913");
  });
});
