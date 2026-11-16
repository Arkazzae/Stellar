import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ImagePart } from "@/lib/messages/parts";

import { ImageAttachment } from "./ImageAttachment";

const measured: ImagePart = {
  kind: "image",
  src: "/assets/cat",
  alt: "A cat",
  width: 1200,
  height: 800,
};

describe("ImageAttachment", () => {
  it("holds the measured box while the image loads, then lets the file size itself", () => {
    render(<ImageAttachment part={measured} />);
    const image = screen.getByRole("img", { name: "A cat" });
    expect(image).toHaveClass("att-image", "reserved");
    expect(image.style.getPropertyValue("--att-image-w")).toBe("1200px");
    expect(image.style.getPropertyValue("--att-image-ratio")).toBe("1200 / 800");

    fireEvent.load(image);
    expect(image).toHaveClass("att-image");
    expect(image).not.toHaveClass("reserved");
    expect(image.style.getPropertyValue("--att-image-w")).toBe("");
  });

  it("gives the box up when the image fails, too", () => {
    render(<ImageAttachment part={measured} />);
    const image = screen.getByRole("img", { name: "A cat" });

    fireEvent.error(image);
    expect(image).not.toHaveClass("reserved");
  });

  it("holds the box again for a different file", () => {
    const { rerender } = render(<ImageAttachment part={measured} />);
    fireEvent.load(screen.getByRole("img", { name: "A cat" }));

    rerender(<ImageAttachment part={{ ...measured, src: "/assets/dog", width: 600, height: 1200 }} />);
    const image = screen.getByRole("img", { name: "A cat" });
    expect(image).toHaveClass("reserved");
    expect(image.style.getPropertyValue("--att-image-ratio")).toBe("600 / 1200");
  });

  it("reserves nothing for an image nobody measured", () => {
    render(<ImageAttachment part={{ kind: "image", src: "/assets/cat", alt: "A cat" }} />);
    const image = screen.getByRole("img", { name: "A cat" });
    expect(image).not.toHaveClass("reserved");
    expect(image).not.toHaveAttribute("style");
  });
});
