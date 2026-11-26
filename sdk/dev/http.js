import { fail } from "./room.js";

export async function readJson(req, limit = 384 * 1024) {
  if (req.headers["content-type"]?.split(";")[0] !== "application/json") fail(415, "Expected JSON.");
  let size = 0;
  const chunks = [];
  for await (const chunk of req) {
    size += chunk.length;
    if (size > limit) fail(413, "Request is too large.");
    chunks.push(chunk);
  }
  try { return JSON.parse(Buffer.concat(chunks)); } catch { fail(400, "Invalid JSON."); }
}
export function json(res, data) {
  res.writeHead(200, { "Content-Type": "application/json", "Cache-Control": "no-store" });
  res.end(JSON.stringify(data));
}
export function problem(res, error) {
  res.writeHead(error.status || 500, { "Content-Type": "text/plain", "Cache-Control": "no-store" });
  res.end(error.status ? error.message : "The preview server could not complete this request.");
}
