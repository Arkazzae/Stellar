import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { NotFoundPage } from "./not-found-page";
import { selectPolish } from "../test/locale";
import { currentPath, visit } from "../test/navigation";

selectPolish();

vi.mock("next/navigation", () => import("../test/navigation"));
vi.mock("next/link", () => import("../test/navigation").then(({ Link }) => ({ default: Link })));

describe("NotFoundPage", () => {
  it("leads back to the overview", async () => {
    visit("/cockpit/nie-ma-takiego-adresu");
    render(<NotFoundPage />);
    expect(screen.getByRole("heading", { name: "Nie ma takiego widoku" })).toBeTruthy();

    const back = screen.getByRole("link", { name: "Wróć do przeglądu" });
    expect(back.getAttribute("href")).toBe("/cockpit");
    await userEvent.click(back);

    expect(currentPath()).toBe("/cockpit");
  });
});
