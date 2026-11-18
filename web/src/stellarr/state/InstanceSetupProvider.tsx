import { usePathname, useRouter } from "@stellarr/router";
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

import { usePorts } from "./PortsProvider";
import { useSessionContext } from "./SessionProvider";

export interface InstanceSetupValue {
  /**
   * True while the instance has no account yet, so the first visitor founds
   * it at /setup. Null until it is known, which is only asked of an anonymous
   * visitor: a signed-in one proves the instance is set up.
   */
  setupRequired: boolean | null;
  /** The question could not be answered; `retry` asks again. */
  failed: boolean;
  retry(): void;
  /** The founding registration went through: the instance is set up from now on. */
  markSetUp(): void;
}

const InstanceSetupContext = createContext<InstanceSetupValue | null>(null);

export function useInstanceSetupContext(): InstanceSetupValue {
  const value = useContext(InstanceSetupContext);
  if (value === null) throw new Error("useInstanceSetup needs an InstanceSetupProvider above it.");
  return value;
}

/**
 * Whether the instance still waits for its owner, asked once per page load,
 * the first time the visitor turns out to be signed out, and not again on
 * every navigation: an instance gets set up once, and the page that does it
 * says so through `markSetUp`.
 *
 * While the answer is yes, every address of the web client but /setup leads
 * there. The redirect never stands in for the page: the page stays mounted,
 * so nothing it holds is lost while a reload of the session runs.
 */
export function InstanceSetupProvider({ children }: { children: ReactNode }) {
  const ports = usePorts();
  const { phase } = useSessionContext();
  const [setupRequired, setSetupRequired] = useState<boolean | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const known = setupRequired !== null;

  useEffect(() => {
    if (phase !== "anonymous" || known) return;
    let active = true;
    ports.session.setupRequired().then(
      (required) => {
        if (!active) return;
        setFailed(false);
        setSetupRequired(required);
      },
      () => {
        if (active) setFailed(true);
      },
    );
    return () => {
      active = false;
    };
  }, [attempt, known, phase, ports]);

  const retry = useCallback(() => {
    setFailed(false);
    setAttempt((count) => count + 1);
  }, []);
  const markSetUp = useCallback(() => setSetupRequired(false), []);

  const value = useMemo(
    () => ({ setupRequired, failed, retry, markSetUp }),
    [failed, markSetUp, retry, setupRequired],
  );

  return (
    <InstanceSetupContext.Provider value={value}>
      <SetupRedirect setupRequired={setupRequired} anonymous={phase === "anonymous"} />
      {children}
    </InstanceSetupContext.Provider>
  );
}

function SetupRedirect({ setupRequired, anonymous }: { setupRequired: boolean | null; anonymous: boolean }) {
  const pathname = usePathname();
  const router = useRouter();
  const away = anonymous && setupRequired === true && pathname !== "/setup";

  useEffect(() => {
    if (away) router.replace("/setup");
  }, [away, router]);

  return null;
}
