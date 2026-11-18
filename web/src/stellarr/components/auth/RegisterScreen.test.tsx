import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { MessageKey } from "@/i18n";
import { polishAsRead } from "@/test/translators";

import { RegisterScreen } from "./RegisterScreen";

const t = polishAsRead;
const field = (key: MessageKey) => screen.getByRole("textbox", { name: t(key) });
const INVITE = { server: "Stellarr Dev", inviter: "Ada" };

describe("RegisterScreen", () => {
  it("suggests a login from the display name until one of the person's own is typed", async () => {
    const user = userEvent.setup();
    render(<RegisterScreen t={t} invite={INVITE} onSubmit={vi.fn()} />);

    await user.type(field("auth.register.displayName"), "Łukasz Żółć");
    expect(field("auth.register.username")).toHaveValue("lukasz.zolc");

    await user.clear(field("auth.register.username"));
    await user.type(field("auth.register.username"), "luk");
    await user.type(field("auth.register.displayName"), "-Nowak");
    expect(field("auth.register.username")).toHaveValue("luk");

    // Emptied, the field goes back to following the name.
    await user.clear(field("auth.register.username"));
    await user.type(field("auth.register.displayName"), "!");
    expect(field("auth.register.username")).toHaveValue("lukasz.zolc-nowak");
  });

  it("walks the problems in visual order and sends the form once it is complete", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn(async () => null);
    render(<RegisterScreen t={t} invite={INVITE} onSubmit={onSubmit} />);
    const send = () => user.click(screen.getByRole("button", { name: t("auth.register.submit") }));

    await send();
    expect(field("auth.register.displayName")).toHaveFocus();
    await user.type(field("auth.register.displayName"), " Ola Nowak ");
    await user.clear(field("auth.register.username"));
    await user.type(field("auth.register.username"), "ol");
    await send();
    expect(field("auth.register.username")).toHaveAttribute("aria-invalid", "true");
    await user.type(field("auth.register.username"), "a");
    await send();
    expect(field("auth.email")).toHaveFocus();
    await user.type(field("auth.email"), "ola@example.com");
    await user.type(screen.getByLabelText(t("auth.password")), "krotkie");
    await send();
    expect(screen.getByRole("alert")).toHaveTextContent(t("auth.password.length"));
    expect(onSubmit).not.toHaveBeenCalled();

    await user.type(screen.getByLabelText(t("auth.password")), "12");
    await send();
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({
      displayName: "Ola Nowak",
      username: "ola",
      email: "ola@example.com",
      password: "krotkie12",
    });
  });

  it("puts what the server refused on its field", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn(async () => ({ field: "username", message: t("auth.register.usernameTaken") }));
    render(<RegisterScreen t={t} invite={INVITE} onSubmit={onSubmit} />);

    await user.type(field("auth.register.displayName"), "Ola");
    await user.type(field("auth.email"), "ola@example.com");
    await user.type(screen.getByLabelText(t("auth.password")), "kawa rower 2026");
    await user.click(screen.getByRole("button", { name: t("auth.register.submit") }));

    expect(await screen.findByRole("alert")).toHaveTextContent(t("auth.register.usernameTaken"));
    expect(field("auth.register.username")).toHaveAttribute("aria-invalid", "true");
    expect(field("auth.register.username")).toHaveFocus();
  });
});
