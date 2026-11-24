import { useEffect, useState } from "react";

import type { SignInProvider, SessionPort } from "@/ports";
import { usePorts } from "@/state/PortsProvider";

/**
 * What the account pages say about the instance before anyone signs in: its
 * name and its identity providers. Each is asked once per page load, by the
 * first page that wants it, and every later page reads the same answer.
 */
interface Asked<T> {
  promise: Promise<T>;
  value?: T;
}

function once<T>(cache: WeakMap<SessionPort, Asked<T>>, session: SessionPort, ask: () => Promise<T>): Asked<T> {
  const cached = cache.get(session);
  if (cached !== undefined) return cached;
  const entry: Asked<T> = { promise: ask() };
  entry.promise.then(
    (value) => {
      entry.value = value;
    },
    // A failure is forgotten, so the next page asks again.
    () => cache.delete(session),
  );
  cache.set(session, entry);
  return entry;
}

function useAsked<T>(cache: WeakMap<SessionPort, Asked<T>>, ask: (session: SessionPort) => Promise<T>): T | null {
  const { session } = usePorts();
  const [value, setValue] = useState<T | null>(() => cache.get(session)?.value ?? null);

  useEffect(() => {
    let active = true;
    once(cache, session, () => ask(session)).promise.then(
      (answer) => active && setValue(answer),
      () => undefined,
    );
    return () => {
      active = false;
    };
  }, [ask, cache, session]);

  return value;
}

const names = new WeakMap<SessionPort, Asked<string>>();
const providers = new WeakMap<SessionPort, Asked<SignInProvider[]>>();
const askName = (session: SessionPort) => session.instanceName();
const askProviders = (session: SessionPort) => session.signInProviders();
const NONE: SignInProvider[] = [];

/** The instance's name, or null until it is known; a page reads a generic line meanwhile. */
export function useInstanceName(): string | null {
  return useAsked(names, askName);
}

/**
 * The identity providers to offer, empty until they are known and when the
 * question fails: the password form works without them.
 */
export function useSignInProviders(): SignInProvider[] {
  return useAsked(providers, askProviders) ?? NONE;
}
