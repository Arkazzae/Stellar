import { fail } from "./room.js";

// Trusted preview hosts exchange a small current image. Reads never enter the activity protocol.
export function createTileRelay(clock = Date.now) {
  const tiles = new Map();
  return (sender, action) => {
    const now = clock();
    for (const [id, entry] of tiles) if (now - entry.at > 1800) tiles.delete(id);
    switch (action?.op) {
      case "clear": tiles.delete(sender); return {};
      case "put": {
        const data = action.image;
        if (typeof data !== "string" || data.length > 96 * 1024 ||
            !/^data:image\/jpeg;base64,[A-Za-z0-9+/]+=*$/.test(data)) fail(400, "Invalid camera preview.");
        const previous = tiles.get(sender);
        if (previous && now - previous.at < 100) fail(429, "Camera is sending too frequently.");
        if (!previous && tiles.size >= 64) fail(429, "Camera preview is full.");
        tiles.set(sender, { at: now, data });
        return {};
      }
      case "get": {
        if (!Array.isArray(action.ids) || action.ids.length > 16) fail(400, "Invalid camera tile list.");
        return Object.fromEntries(action.ids.map(id => [id, tiles.get(id)?.data ?? null]));
      }
      default: fail(400, "Unknown camera request.");
    }
  };
}
