import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { encodePlaceholder, placeholderToBase64 } from "@/lib/media/placeholder";

import { ImageAttachment } from "./ImageAttachment";

const stand = placeholderToBase64(
  encodePlaceholder({ width: 2, height: 2, data: new Uint8ClampedArray(16).fill(255) }),
);

describe("ImageAttachment placeholder", () => {
  it("paints the stand-in until the image has loaded, then drops it", () => {
    render(
      <ImageAttachment part={{ kind: "image", src: "/assets/a", placeholder: stand, alt: "Kot" }} />,
    );
    const image = screen.getByRole("img", { name: "Kot" });
    expect(image.style.backgroundImage).toContain("data:image/png;base64,");

    fireEvent.load(image);
    expect(image.style.backgroundImage).toBe("");
  });

  it("paints a gallery tile the same way and draws nothing for bytes that do not decode", () => {
    render(
      <ImageAttachment
        part={{
          kind: "gallery",
          images: [
            { src: "/assets/a", placeholder: stand, alt: "Pierwszy" },
            { src: "/assets/b", placeholder: "!!!!", alt: "Drugi" },
          ],
        }}
      />,
    );
    const first = screen.getByRole("img", { name: "Pierwszy" });
    expect(first.style.backgroundImage).toContain("data:image/png;base64,");
    expect(screen.getByRole("img", { name: "Drugi" }).style.backgroundImage).toBe("");

    fireEvent.load(first);
    expect(first.style.backgroundImage).toBe("");
  });
});

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
