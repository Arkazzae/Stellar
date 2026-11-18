/**
 * Whether this browser can take part in encrypted conversations.
 *
 * WebCrypto exists only in a secure context (HTTPS or localhost), and X25519
 * arrived in it later than the rest, so the probe generates one pair rather
 * than trusting a feature list. The desktop and Android shells embed web views
 * whose engines vary, and the same probe decides there.
 */
export type E2eeSupport = "supported" | "insecure" | "unsupported";

let probe: Promise<E2eeSupport> | null = null;

async function run(): Promise<E2eeSupport> {
  if (typeof window !== "undefined" && window.isSecureContext === false) return "insecure";
  const subtle = globalThis.crypto?.subtle as SubtleCrypto | undefined;
  if (subtle === undefined) return "insecure";
  if (typeof indexedDB === "undefined") return "unsupported";
  try {
    await subtle.generateKey({ name: "X25519" }, false, ["deriveBits"]);
    return "supported";
  } catch {
    return "unsupported";
  }
}

/** Probes once per page; every caller shares the answer. */
export function probeE2eeSupport(): Promise<E2eeSupport> {
  probe ??= run();
  return probe;
}
