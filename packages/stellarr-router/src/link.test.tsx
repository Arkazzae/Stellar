import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Children, cloneElement, createRef, isValidElement, type MouseEvent, type ReactElement, type ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import { createBrowserHistory, type RouterHistory } from "./history";
import { Link } from "./link";
import { createMemoryHistory, type MemoryHistory } from "./memory";
import { Outlet, Router } from "./router";
import { defineRoutes } from "./routes";

const routes = defineRoutes({
  routes: [
    { path: "/", page: () => <h1>Home</h1> },
    { path: "/login", page: () => <h1>Login</h1> },
  ],
  notFound: () => <h1>Not found</h1>,
});

function renderLink<History extends RouterHistory = MemoryHistory>(
  link: ReactNode,
  history: History = createMemoryHistory("/") as RouterHistory as History,
): History {
  render(
    <Router routes={routes} history={history}>
      {link}
      <Outlet />
    </Router>,
  );
  return history;
}

/**
 * Clicks the link and says whether the browser would still follow it. The
 * answer is read after the app's handlers ran, and the click is then stopped,
 * as jsdom cannot load another document.
 */
function click(name: string, init: Partial<globalThis.MouseEventInit> = {}): boolean {
  let followed = false;
  const settle = (event: Event) => {
    followed = !event.defaultPrevented;
    event.preventDefault();
  };
  document.addEventListener("click", settle, { once: true });
  fireEvent.click(screen.getByRole("link", { name }), { button: 0, ...init });
  return followed;
}

describe("Link", () => {
  it("renders a real link", () => {
    renderLink(<Link href="/login" className="nav">Sign in</Link>);

    const link = screen.getByRole("link", { name: "Sign in" });
    expect(link).toHaveAttribute("href", "/login");
    expect(link).toHaveClass("nav");
  });

  it("opens an address of the app without a page load", async () => {
    const history = renderLink(<Link href="/login">Sign in</Link>);

    await userEvent.click(screen.getByRole("link", { name: "Sign in" }));

    expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument();
    expect(history.navigations).toEqual([{ kind: "push", href: "/login" }]);
  });

  it("replaces the entry when asked", () => {
    const history = createMemoryHistory("/");
    renderLink(
      <Link href="/login" replace>
        Sign in
      </Link>,
      history,
    );

    expect(click("Sign in")).toBe(false);
    expect(history.navigations).toEqual([{ kind: "replace", href: "/login" }]);
  });

  it.each([
    ["the meta key", { metaKey: true }],
    ["the control key", { ctrlKey: true }],
    ["the shift key", { shiftKey: true }],
    ["the alt key", { altKey: true }],
    ["the middle button", { button: 1 }],
  ])("leaves a click with %s to the browser", (_case, init) => {
    const history = createMemoryHistory("/");
    renderLink(<Link href="/login">Sign in</Link>, history);

    expect(click("Sign in", init)).toBe(true);
    expect(history.navigations).toEqual([]);
  });

  it("leaves a link to another tab or a download to the browser", () => {
    const history = createMemoryHistory("/");
    renderLink(
      <>
        <Link href="/login" target="_blank">
          New tab
        </Link>
        <Link href="/login" download>
          Download
        </Link>
        <Link href="/login" target="_self">
          Same tab
        </Link>
      </>,
      history,
    );

    expect(click("New tab")).toBe(true);
    expect(click("Download")).toBe(true);
    expect(history.navigations).toEqual([]);
    expect(click("Same tab")).toBe(false);
  });

  it("leaves another app on this origin and another site to the browser", () => {
    window.history.replaceState(null, "", "/");
    renderLink(
      <>
        <Link href="/cockpit">Cockpit</Link>
        <Link href="https://docs.example.test/">Docs</Link>
        <Link href="mailto:owner@example.test">Mail</Link>
      </>,
      createBrowserHistory(window),
    );

    expect(click("Cockpit")).toBe(true);
    expect(click("Docs")).toBe(true);
    expect(click("Mail")).toBe(true);
    expect(screen.getByRole("heading", { name: "Home" })).toBeInTheDocument();
  });

  it("records a page load in a test's history instead of following it", () => {
    const history = createMemoryHistory("/login");
    renderLink(
      <>
        <Link href="/">Home</Link>
        <Link href="/cockpit/users">Cockpit</Link>
      </>,
      history,
    );

    expect(click("Cockpit")).toBe(false);
    expect(history.navigations).toEqual([{ kind: "leave", href: "/cockpit/users", replace: false }]);
  });

  it("leaves a link that changes only the fragment to the browser", () => {
    const history = createMemoryHistory("/login");
    renderLink(<Link href="#code">Skip</Link>, history);

    expect(click("Skip")).toBe(true);
    expect(history.navigations).toEqual([]);
  });

  it("runs the caller's handler first, and does nothing when it prevented the default", () => {
    const history = createMemoryHistory("/");
    const seen = vi.fn((event: MouseEvent<HTMLAnchorElement>) => event.preventDefault());
    renderLink(
      <Link href="/login" onClick={seen}>
        Sign in
      </Link>,
      history,
    );

    expect(click("Sign in")).toBe(false);
    expect(seen).toHaveBeenCalledOnce();
    expect(history.navigations).toEqual([]);
  });

  it("is a plain link without a router", () => {
    render(<Link href="/login">Sign in</Link>);

    expect(screen.getByRole("link", { name: "Sign in" })).toHaveAttribute("href", "/login");
    expect(click("Sign in")).toBe(true);
  });

  it("hands its element to a ref", () => {
    const ref = createRef<HTMLAnchorElement>();
    renderLink(
      <Link href="/login" ref={ref}>
        Sign in
      </Link>,
    );

    expect(ref.current).toBe(screen.getByRole("link", { name: "Sign in" }));
  });

  it("works as the child of an asChild component, which merges its props in", async () => {
    // As @stellarr/ui's Slot does it: the child's handler first, then the
    // component's, and the classes joined.
    function AsChild({ children, onClick }: { children: ReactNode; onClick: () => void }) {
      const child = Children.only(children) as ReactElement<{ className?: string; onClick?: (event: unknown) => void }>;
      if (!isValidElement(child)) return null;
      return cloneElement(child, {
        className: `button ${child.props.className ?? ""}`.trim(),
        onClick: (event: unknown) => {
          child.props.onClick?.(event);
          onClick();
        },
      });
    }
    const pressed = vi.fn();
    const history = renderLink(
      <AsChild onClick={pressed}>
        <Link href="/login">Sign in</Link>
      </AsChild>,
    );

    await userEvent.click(screen.getByRole("link", { name: "Sign in" }));

    expect(screen.getByRole("link", { name: "Sign in" })).toHaveClass("button");
    expect(pressed).toHaveBeenCalledOnce();
    expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument();
    expect(history.navigations).toEqual([{ kind: "push", href: "/login" }]);
  });
});
