import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { lazy, useLayoutEffect, useState, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createBrowserHistory, type RouterHistory } from "./history";
import { createMemoryHistory } from "./memory";
import { Outlet, Router, useParams, usePathname, useRouter, useSearchParams } from "./router";
import { defineRoutes } from "./routes";

function Home() {
  const router = useRouter();
  return (
    <main>
      <h1>Home</h1>
      <button type="button" onClick={() => router.push("/login?fresh=1")}>
        Sign in
      </button>
      <button type="button" onClick={() => router.replace("/login")}>
        Redirect
      </button>
      <button type="button" onClick={() => router.push("/")}>
        Here
      </button>
      <button type="button" onClick={() => router.push("/cockpit/")}>
        Cockpit
      </button>
      <button type="button" onClick={() => router.replace("https://docs.example.test/")}>
        Docs
      </button>
    </main>
  );
}

function Login() {
  const query = useSearchParams();
  const router = useRouter();
  const [count, setCount] = useState(0);
  return (
    <main>
      <h1>Login</h1>
      <p>at {usePathname()}</p>
      <p>fresh: {query.get("fresh") ?? "none"}</p>
      <button type="button" onClick={() => setCount(count + 1)}>
        Pressed {count}
      </button>
      <button type="button" onClick={() => router.push("/login?fresh=2")}>
        Again
      </button>
      <button type="button" onClick={() => router.push("/invite/abc")}>
        Invite
      </button>
    </main>
  );
}

function Invite() {
  const { code } = useParams<"code">();
  return <h1>{code === null ? "Unreadable invite" : `Invite ${code}`}</h1>;
}

function NotFound() {
  return <h1>Not found</h1>;
}

const routes = defineRoutes({
  routes: [
    { path: "/", page: Home },
    { path: "/login", page: Login },
    { path: "/invite/:code", page: Invite },
  ],
  notFound: NotFound,
});

function renderAt(history: RouterHistory, children: ReactNode = <Outlet />) {
  return render(
    <Router routes={routes} history={history}>
      {children}
    </Router>,
  );
}

describe("Router and Outlet", () => {
  it("shows the page the table gives the address", () => {
    renderAt(createMemoryHistory("/login?fresh=1"));

    expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument();
    expect(screen.getByText("at /login")).toBeInTheDocument();
    expect(screen.getByText("fresh: 1")).toBeInTheDocument();
  });

  it("shows the not-found page for an address no route claims", () => {
    renderAt(createMemoryHistory("/nope"));

    expect(screen.getByRole("heading", { name: "Not found" })).toBeInTheDocument();
  });

  it("gives a page its parameters decoded, and null for one that cannot be decoded", () => {
    const { unmount } = renderAt(createMemoryHistory(`/invite/${encodeURIComponent("a b")}`));
    expect(screen.getByRole("heading", { name: "Invite a b" })).toBeInTheDocument();
    unmount();

    renderAt(createMemoryHistory("/invite/%E0%A4%A"));
    expect(screen.getByRole("heading", { name: "Unreadable invite" })).toBeInTheDocument();
  });

  it("answers a trailing slash with the canonical path", () => {
    renderAt(createMemoryHistory("/login/"));

    expect(screen.getByText("at /login")).toBeInTheDocument();
  });

  it("pushes a new entry and goes back to the one before", async () => {
    const user = userEvent.setup();
    const history = createMemoryHistory("/");
    renderAt(history);

    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(screen.getByText("fresh: 1")).toBeInTheDocument();
    expect(history.navigations).toEqual([{ kind: "push", href: "/login?fresh=1" }]);

    act(() => history.back());
    expect(screen.getByRole("heading", { name: "Home" })).toBeInTheDocument();
    act(() => history.forward());
    expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument();
  });

  it("replaces the entry for a redirect", async () => {
    const user = userEvent.setup();
    const history = createMemoryHistory("/");
    renderAt(history);

    await user.click(screen.getByRole("button", { name: "Redirect" }));

    expect(history.navigations).toEqual([{ kind: "replace", href: "/login" }]);
    expect(history.entries).toEqual(["/login"]);
  });

  it("changes nothing when asked for the address already shown", async () => {
    const user = userEvent.setup();
    const history = createMemoryHistory("/");
    renderAt(history);

    await user.click(screen.getByRole("button", { name: "Here" }));

    expect(history.navigations).toEqual([]);
  });

  it("loads a page for an address outside its table, in place when replacing", async () => {
    const user = userEvent.setup();
    const history = createMemoryHistory("/");
    renderAt(history);

    await user.click(screen.getByRole("button", { name: "Cockpit" }));
    await user.click(screen.getByRole("button", { name: "Docs" }));

    expect(history.navigations).toEqual([
      { kind: "leave", href: "/cockpit", replace: false },
      { kind: "leave", href: "https://docs.example.test/", replace: true },
    ]);
    expect(screen.getByRole("heading", { name: "Home" })).toBeInTheDocument();
  });

  it("keeps a page's state when only the query changes, and starts afresh on a new path", async () => {
    const user = userEvent.setup();
    const history = createMemoryHistory("/login");
    renderAt(history);

    await user.click(screen.getByRole("button", { name: "Pressed 0" }));
    await user.click(screen.getByRole("button", { name: "Again" }));
    expect(screen.getByText("fresh: 2")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pressed 1" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Invite" }));
    act(() => history.back());
    expect(screen.getByRole("button", { name: "Pressed 0" })).toBeInTheDocument();
  });

  it("hears a navigation made before it subscribed, by a child's layout effect", () => {
    function RedirectAtOnce() {
      const router = useRouter();
      useLayoutEffect(() => router.replace("/login"), [router]);
      return null;
    }
    const history = createMemoryHistory("/");

    renderAt(
      history,
      <>
        <RedirectAtOnce />
        <Outlet />
      </>,
    );

    expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument();
  });

  it("shows the fallback while a page's code loads the first time, and the old page on later visits", async () => {
    let finish: (module: { default: () => ReactNode }) => void = () => undefined;
    const Slow = lazy(() => new Promise<{ default: () => ReactNode }>((resolve) => (finish = resolve)));
    const table = defineRoutes({
      routes: [
        { path: "/", page: Home },
        { path: "/slow", page: Slow },
      ],
      notFound: NotFound,
    });
    const history = createMemoryHistory("/");
    render(
      <Router routes={table} history={history}>
        <Outlet fallback={<p>Loading</p>} />
      </Router>,
    );

    act(() => history.push({ pathname: "/slow", search: "", hash: "" }));
    // The transition keeps the page that is ready.
    expect(screen.getByRole("heading", { name: "Home" })).toBeInTheDocument();
    expect(screen.queryByText("Loading")).not.toBeInTheDocument();

    await act(async () => finish({ default: () => <h1>Slow</h1> }));
    expect(await screen.findByRole("heading", { name: "Slow" })).toBeInTheDocument();
  });

  it("shows the fallback when the first page's code is not there yet", async () => {
    let finish: (module: { default: () => ReactNode }) => void = () => undefined;
    const Slow = lazy(() => new Promise<{ default: () => ReactNode }>((resolve) => (finish = resolve)));
    const table = defineRoutes({ routes: [{ path: "/slow", page: Slow }], notFound: NotFound });

    render(
      <Router routes={table} history={createMemoryHistory("/slow")}>
        <Outlet fallback={<p>Loading</p>} />
      </Router>,
    );

    expect(screen.getByText("Loading")).toBeInTheDocument();
    await act(async () => finish({ default: () => <h1>Slow</h1> }));
    expect(screen.getByRole("heading", { name: "Slow" })).toBeInTheDocument();
  });

  it("asks for a router above every hook", () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    function Orphan() {
      usePathname();
      return null;
    }

    expect(() => render(<Orphan />)).toThrow(/needs a <Router>/);
  });
});

describe("Router with the browser's history", () => {
  let scrollY = 0;

  beforeEach(() => {
    window.history.replaceState(null, "", "/");
    scrollY = 0;
    vi.spyOn(window, "scrollY", "get").mockImplementation(() => scrollY);
    window.scrollTo = vi.fn((x: number, y: number) => {
      scrollY = y;
    }) as unknown as typeof window.scrollTo;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    (document.activeElement as HTMLElement | null)?.blur();
  });

  it("scrolls to the top for a new page, and leaves the window alone for a new query", async () => {
    const user = userEvent.setup();
    renderAt(createBrowserHistory(window));
    scrollY = 300;

    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(window.scrollTo).toHaveBeenLastCalledWith(0, 0);

    scrollY = 120;
    vi.mocked(window.scrollTo).mockClear();
    await user.click(screen.getByRole("button", { name: "Again" }));
    expect(screen.getByText("fresh: 2")).toBeInTheDocument();
    expect(window.scrollTo).not.toHaveBeenCalled();
  });

  it("returns the window to where an entry was left when going back", async () => {
    const user = userEvent.setup();
    renderAt(createBrowserHistory(window));
    scrollY = 300;
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    scrollY = 40;

    window.history.back();

    await screen.findByRole("heading", { name: "Home" });
    expect(window.scrollTo).toHaveBeenLastCalledWith(0, 300);
  });

  it("hands the focus to the new page's heading when it went down with the old page", async () => {
    const user = userEvent.setup();
    renderAt(createBrowserHistory(window));

    await user.click(screen.getByRole("button", { name: "Sign in" }));

    const heading = screen.getByRole("heading", { name: "Login" });
    expect(heading).toHaveFocus();
    expect(heading).toHaveAttribute("tabindex", "-1");
  });

  it("leaves the focus alone after a redirect", async () => {
    const user = userEvent.setup();
    renderAt(createBrowserHistory(window));

    await user.click(screen.getByRole("button", { name: "Redirect" }));

    expect(screen.getByRole("heading", { name: "Login" })).not.toHaveFocus();
    expect(window.scrollTo).toHaveBeenLastCalledWith(0, 0);
  });

  it("leaves the focus where it was when that element outlived the change", async () => {
    const user = userEvent.setup();
    const history = createBrowserHistory(window);
    function Shell() {
      const router = useRouter();
      return (
        <>
          <nav>
            <button type="button" onClick={() => router.push("/login")}>
              Persistent
            </button>
          </nav>
          <Outlet />
        </>
      );
    }
    renderAt(history, <Shell />);

    await user.click(screen.getByRole("button", { name: "Persistent" }));

    await waitFor(() => expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Persistent" })).toHaveFocus();
  });
});
