/// <reference types="vite/client" />

/** The variables the build writes into the web client. */
interface ImportMetaEnv {
  /** Optional development STUN servers as a JSON array. */
  readonly VITE_ICE_SERVERS?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

/** The size of each prebuilt file by the path it is served under; see `artifactSizes.ts`. */
declare module "virtual:artifact-sizes" {
  const sizes: Readonly<Record<string, number>>;
  export default sizes;
}
