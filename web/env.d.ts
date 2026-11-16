/// <reference types="vite/client" />

/** The variables the build writes into the web client. */
interface ImportMetaEnv {
  /** ICE servers as a JSON array, for an instance without coturn (`src/stellarr/lib/platform/env.ts`). */
  readonly VITE_ICE_SERVERS?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
