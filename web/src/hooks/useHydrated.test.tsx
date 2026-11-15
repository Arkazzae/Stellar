import { act, render, screen } from "@testing-library/react";
import { hydrateRoot } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { useHydrated } from "./useHydrated";

function Probe() {
  return <output aria-label="hydrated">{String(useHydrated())}</output>;
}

describe("useHydrated", () => {
  it("is false in a render made ahead of time", () => {
    expect(renderToString(<Probe />)).toContain(">false</output>");
  });

  it("is true at once in a render made only in the browser", () => {
    render(<Probe />);
    expect(screen.getByLabelText("hydrated")).toHaveTextContent("true");
  });

  it("hydrates the earlier render as it was, then turns true", async () => {
    const container = document.createElement("div");
    container.innerHTML = renderToString(<Probe />);
    document.body.append(container);
    const recoverable = vi.fn();

    await act(async () => {
      hydrateRoot(container, <Probe />, { onRecoverableError: recoverable });
    });

    expect(recoverable).not.toHaveBeenCalled();
    expect(container.querySelector("output")).toHaveTextContent("true");
    container.remove();
  });
});
