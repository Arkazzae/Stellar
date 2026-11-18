/**
 * Fetches the socket and voice implementations that only the signed-in app
 * uses (`signedInCode.ts`). `loadApp` asks for it together with the app, so
 * whenever the app is on screen the code is there and `loadedSignedInCode`
 * can hand it out without waiting.
 */

import type * as SignedInModule from "./signedInCode";

type SignedInCode = typeof SignedInModule;

let loaded: SignedInCode | null = null;
let loading: Promise<SignedInCode> | null = null;

/** One request however many callers; a failed one is forgotten so the next caller tries again. */
export function loadSignedInCode(): Promise<SignedInCode> {
  if (loading !== null) return loading;
  const request = import("./signedInCode").then(
    (code) => {
      loaded = code;
      return code;
    },
    (cause: unknown) => {
      loading = null;
      throw cause;
    },
  );
  loading = request;
  return request;
}

/** The code, which must have been loaded already. */
export function loadedSignedInCode(): SignedInCode {
  if (loaded === null) {
    throw new Error("The signed-in code is not loaded yet.");
  }
  return loaded;
}
