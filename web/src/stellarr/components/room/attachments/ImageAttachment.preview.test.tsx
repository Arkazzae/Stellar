import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ImageAttachment } from "./ImageAttachment";

describe("ImageAttachment with a preview", () => {
  it("draws the preview of a lone image and opens the original", () => {
    const onOpen = vi.fn();
    render(
      <ImageAttachment
        part={{ kind: "image", src: "/assets/a", preview: "/assets/a/preview", alt: "Kot" }}
        onOpen={onOpen}
      />,
    );

    const image = screen.getByRole("img", { name: "Kot" });
    expect(image).toHaveAttribute("src", "/assets/a/preview");
    fireEvent.click(image);
    expect(onOpen).toHaveBeenCalledWith(
      [{ src: "/assets/a", alt: "Kot", preview: "/assets/a/preview" }],
      0,
    );
  });

  it("draws each gallery tile from its preview where it has one", () => {
    render(
      <ImageAttachment
        part={{
          kind: "gallery",
          images: [
            { src: "/assets/a", preview: "/assets/a/preview", alt: "Pierwszy" },
            { src: "/assets/b", alt: "Drugi" },
          ],
        }}
      />,
    );

    expect(screen.getByRole("img", { name: "Pierwszy" })).toHaveAttribute("src", "/assets/a/preview");
    expect(screen.getByRole("img", { name: "Drugi" })).toHaveAttribute("src", "/assets/b");
  });
});
