import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { UserAvatar } from "./user-avatar";

describe("UserAvatar", () => {
  it("shows the initials and no picture", () => {
    const { container } = render(<UserAvatar displayName="Ola Owner" />);

    expect(container.textContent).toBe("OO");
    expect(container.querySelector("img")).toBeNull();
  });
});
