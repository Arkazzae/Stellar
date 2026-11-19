import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { defaultIdentityArt } from "@/design/identity";

import { Avatar } from "./Avatar";

describe("Avatar", () => {
  it("hides the initials behind the built-in mascot until it fails to load", () => {
    const { container, rerender } = render(
      <Avatar id="person-42" displayName="Patrycja Nowak" />,
    );
    const picture = container.querySelector("img")!;

    expect(picture).toHaveAttribute(
      "src",
      defaultIdentityArt("person-42").avatarUrl,
    );
    expect(screen.getByText("PN")).not.toBeVisible();

    fireEvent.error(picture);
    expect(screen.getByText("PN")).toBeVisible();

    // The same face handed to someone else tries their mascot afresh.
    expect(defaultIdentityArt("person-7").avatarUrl).not.toBe(
      defaultIdentityArt("person-42").avatarUrl,
    );
    rerender(<Avatar id="person-7" displayName="Patrycja Nowak" />);
    expect(screen.getByText("PN")).not.toBeVisible();
  });

  it("prefers an uploaded avatar and keeps the initials under it", () => {
    const { container } = render(
      <Avatar
        id="person-42"
        displayName="Patrycja Nowak"
        avatarUrl="/assets/uploaded-avatar.webp"
      />,
    );

    expect(container.querySelector("img")).toHaveAttribute(
      "src",
      "/assets/uploaded-avatar.webp",
    );
    expect(screen.getByText("PN")).toBeVisible();
  });

  it("keeps system notices as glyphs", () => {
    const { container } = render(
      <Avatar id="system" displayName="System" initials="S" system />,
    );

    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toBe("S");
  });

  it("shows a server icon for a system notice when one is available", () => {
    const { container } = render(
      <Avatar
        id="system"
        displayName="Orbit"
        avatarUrl="/assets/orbit.webp"
        system
      />,
    );

    expect(container.querySelector("img")).toHaveAttribute(
      "src",
      "/assets/orbit.webp",
    );
    expect(container.textContent).toContain("O");
  });
});
