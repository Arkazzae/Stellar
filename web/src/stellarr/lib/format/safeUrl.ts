const UNSAFE = /[\\\u0000-\u001f\u007f]/u;

function parse(value: string | undefined, origin?: string): URL | undefined {
  if (!value || UNSAFE.test(value) || value !== value.trim()) return undefined;
  try {
    if (UNSAFE.test(decodeURI(value))) return undefined;
    return new URL(value, origin);
  } catch {
    return undefined;
  }
}

export function safeHttpUrl(value: string | undefined): string | undefined {
  const url = parse(value);
  if (url && /^https?:\/\//i.test(value ?? "") && !url.username && !url.password) return value;
  return undefined;
}

export function safeAssetUrl(value: string | undefined): string | undefined {
  const origin = globalThis.location?.origin ?? "https://stellarr.invalid";
  if (!value || value.startsWith("//") || (!value.startsWith("/") && !safeHttpUrl(value))) return undefined;
  const url = parse(value, origin);
  return url?.origin === origin && /^\/(?:assets|api)\//.test(url.pathname) && !/%2f/i.test(url.pathname)
    ? value
    : undefined;
}

export function safeAttachmentUrl(value: string | undefined): string | undefined {
  if (value?.startsWith("blob:")) {
    const origin = globalThis.location?.origin;
    return origin && parse(value)?.origin === origin ? value : undefined;
  }
  if (value && /^e2ee:[A-Za-z0-9_-]+$/.test(value)) return value;
  return safeAssetUrl(value);
}
