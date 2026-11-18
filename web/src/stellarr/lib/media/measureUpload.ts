import type { UploadMediaMetadata } from "./probe";

/**
 * Measures a file about to be uploaded. The probe, with the preview maker and
 * the placeholder codec behind it, is loaded only when a file is uploaded, so
 * none of it weighs on the first page. A probe that cannot be loaded is like
 * one that cannot read the file: the file uploads without the measurements.
 */
export async function measureUpload(file: File): Promise<UploadMediaMetadata> {
  try {
    const { probeMedia } = await import("./probe");
    return await probeMedia(file);
  } catch {
    return {};
  }
}
