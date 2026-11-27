// Offline stand-ins for GIPHY in `stellarr dev`: no network and no key.
// Results are the same for every player, and an id carries its own label,
// so another tab can load a pick it never searched for.
const total = 48;
const colors = ["#c2410c", "#1d4ed8", "#15803d", "#7e22ce", "#b91c1c", "#0f766e", "#a16207", "#be185d"];
const shapes = [[480, 270], [360, 360], [300, 400]];
const encoder = new TextEncoder(), decoder = new TextDecoder();
const failure = (code, message) => Object.assign(new Error(message), { code });
const escape = text => text.replace(/[<>&"]/g, char => "&#" + char.charCodeAt(0) + ";");

function shorten(label) {
  let text = "";
  for (const char of label) {
    if (encoder.encode(text + char).length > 24) break;
    text += char;
  }
  return text;
}
const hex = text => Array.from(encoder.encode(text), byte => byte.toString(16).padStart(2, "0")).join("");
function parse(id) {
  const match = /^fx(\d{1,2})q((?:[0-9a-f]{2}){0,24})$/.exec(id);
  if (!match || Number(match[1]) >= total) return null;
  const bytes = Uint8Array.from(match[2].match(/../g) ?? [], pair => parseInt(pair, 16));
  return { index: Number(match[1]), label: decoder.decode(bytes) };
}
function describe(label, index) {
  const [width, height] = shapes[index % shapes.length];
  return { id: "fx" + index + "q" + hex(label), alt: (label || "trending") + " #" + (index + 1), width, height };
}
function picture({ label, index }, size) {
  const item = describe(label, index);
  const scale = size === "full" ? 1 : 200 / item.width;
  const width = Math.round(item.width * scale), height = Math.round(item.height * scale);
  const color = colors[(encoder.encode(label).reduce((sum, byte) => sum + byte, 0) + index) % colors.length];
  const r = Math.round(Math.min(width, height) / 9);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">` +
    `<rect width="100%" height="100%" fill="${color}"/>` +
    `<circle cx="${width / 2}" cy="${Math.round(height * 0.68)}" r="${r}" fill="#fff" opacity=".85">` +
    `<animate attributeName="r" values="${r};${Math.round(r * 1.5)};${r}" dur="1.2s" repeatCount="indefinite"/></circle>` +
    `<text x="50%" y="38%" fill="#fff" font-family="system-ui,sans-serif" font-size="${Math.round(height / 9)}" ` +
    `text-anchor="middle">${escape(item.alt)}</text></svg>`;
  const bytes = encoder.encode(svg);
  return { bytes: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), type: "image/svg+xml",
    width, height, alt: item.alt };
}

/**
 * `mode()` picks what the activity hears: "on", "declined" (this player
 * turned GIPHY off) or "unavailable" (the instance offers no GIFs).
 */
export function createGifFixtures({ mode = () => "on" } = {}) {
  function check() {
    const current = mode();
    if (current === "declined") throw failure("declined", "This player turned GIPHY off.");
    if (current === "unavailable") throw failure("unavailable", "GIFs are off on this instance.");
  }
  return {
    async search({ query, offset, limit }) {
      check();
      const label = shorten(query);
      const items = [];
      for (let index = offset; index < Math.min(total, offset + limit); index++) items.push(describe(label, index));
      return { items, offset, total };
    },
    async load({ gif, size }) {
      check();
      const known = parse(gif);
      if (!known) throw failure("notFound", "The simulator has no such GIF.");
      return picture(known, size);
    },
  };
}
