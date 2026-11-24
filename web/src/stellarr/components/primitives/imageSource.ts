/** An image over HTTP(S), or an object URL used by a local upload preview. */
export function imageSource(source: string | null | undefined): string | undefined {
  if (source == null || source.trim() === "") return undefined;

  let url: URL;
  try {
    url = new URL(source, document.baseURI);
  } catch {
    return undefined;
  }

  if (url.protocol === "blob:") return `blob:${url.pathname}`;

  if (url.protocol !== "https:" && url.protocol !== "http:") return undefined;

  const path = `${url.pathname}${url.search}${url.hash}`;
  if (source.startsWith("/") && url.origin === window.location.origin && !path.startsWith("//")) {
    return `/${path.slice(1)}`;
  }

  return url.protocol === "https:" ? `https://${url.host}${path}` : `http://${url.host}${path}`;
}
