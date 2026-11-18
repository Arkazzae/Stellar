/** The file name the upload form gives a preview, by what it was encoded as. */
export function previewFileName(preview: Blob): string {
  return preview.type === "image/webp" ? "preview.webp" : "preview.jpg";
}
