import type { MessageKey, Translator } from "@/i18n";

import { ActivityRequestError, type DeviceClass } from "./api";

/** Why an activity does not run here, in words: never the raw device ids. */
export const UNSUPPORTED_ON: Record<DeviceClass, MessageKey> = {
  phone: "activities.unsupported.phone",
  tablet: "activities.unsupported.tablet",
  desktop: "activities.unsupported.desktop",
};

/**
 * Why starting or joining an activity failed, as the person reads it. The
 * server's reason wins; only a failure it did not explain is the general one.
 */
export function refusalText(t: Translator, cause: unknown, device: DeviceClass): string {
  if (!(cause instanceof ActivityRequestError)) return t("activities.error.generic");
  switch (cause.code) {
    case "full":
      return t("activities.error.full");
    case "unsupported-device":
      return t(UNSUPPORTED_ON[device]);
    case "not-found":
      return t("activities.error.notFound");
    case "activity-running":
      return t("activities.error.running");
    case "not-host":
      return t("activities.error.notHost");
    case null:
      break;
  }
  if (cause.status === 0) return t("activities.error.offline");
  if (cause.status === 403) return t("activities.error.seat");
  return t("activities.error.generic");
}
