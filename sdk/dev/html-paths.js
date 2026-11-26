// In these elements the browser treats apparent start tags as text.
const textElements = new Set(["script", "style", "title", "textarea", "iframe", "noscript", "noembed", "noframes", "xmp"]);

function rootRelativeUrls(html) {
  const urls = new Set();
  const startTag = /<!--[\s\S]*?(?:--!?>|$)|<![^>]*>|<\?[^>]*>|<([a-z][\w.:-]*)(?=[\s/>])/gi;
  // Consume every attribute, including quoted values containing spaces or '>'.
  const attribute = /\s+|\/+|>|([^\s/>=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?|=/gy;
  let tag;
  while ((tag = startTag.exec(html)) !== null) {
    if (!tag[1]) continue;
    attribute.lastIndex = startTag.lastIndex;
    let token;
    while ((token = attribute.exec(html)) !== null && token[0] !== ">") {
      const name = token[1]?.toLowerCase();
      if (name !== "src" && name !== "href") continue;
      const url = (token[2] ?? token[3] ?? token[4] ?? "").trim();
      if (url.startsWith("/") && !url.startsWith("//") && !url.startsWith("/_stellarr/")) urls.add(url);
    }
    if (!token) break;
    startTag.lastIndex = attribute.lastIndex;
    const element = tag[1].toLowerCase();
    if (element === "plaintext") break;
    if (textElements.has(element)) {
      const endTag = new RegExp(`</${element}(?=[\\s/>])[^>]*>`, "gi");
      endTag.lastIndex = startTag.lastIndex;
      if (!endTag.exec(html)) break;
      startTag.lastIndex = endTag.lastIndex;
    }
  }
  return urls;
}

export function warnAboutHtmlPaths(assets) {
  for (const [file, bytes] of assets) {
    if (!file.toLowerCase().endsWith(".html")) continue;
    for (const url of rootRelativeUrls(bytes.toString("utf8"))) {
      console.warn(`Warning in ${file}: ${JSON.stringify(url)} resolves from the instance root. Use a package-relative URL. For Vite builds, set base: "./".`);
    }
  }
}
