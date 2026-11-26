interface InstallPrompt extends Event {
  prompt(): Promise<void>;
  userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
}

let available: InstallPrompt | null = null;
let installed = false;
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((listener) => listener());

/** Capture the one-use prompt before the lazy signed-in shell loads. */
export function observeInstallation() {
  window.addEventListener("beforeinstallprompt", (event) => {
    event.preventDefault();
    available = event as InstallPrompt;
    installed = false;
    notify();
  });
  window.addEventListener("appinstalled", () => {
    installed = true;
    available = null;
    notify();
  });
  window.matchMedia("(display-mode: standalone)").addEventListener("change", notify);
}

export function subscribeInstallation(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function installationMode() {
  const device = navigator as Navigator & { standalone?: boolean };
  if (installed || device.standalone || window.matchMedia("(display-mode: standalone)").matches) return "installed";
  if (!window.isSecureContext) return "insecure";
  if (available) return "prompt";
  if (/iPad|iPhone|iPod/.test(device.userAgent) || (device.platform === "MacIntel" && device.maxTouchPoints > 1)) return "ios";
  if (/Macintosh/.test(device.userAgent) && /Version\/.+Safari\//.test(device.userAgent)) return "mac";
  return "menu";
}

export async function promptInstallation() {
  const event = available;
  if (!event) return;
  available = null;
  notify();
  try {
    await event.prompt();
    if ((await event.userChoice).outcome === "accepted") installed = true;
  } finally {
    notify();
  }
}

const INVITE_OFFER = "stellarr.offer-install";
const OFFER_SEEN = "stellarr.install-offered";

export function offerInstallAfterInvite() {
  try {
    if (!localStorage.getItem(OFFER_SEEN)) sessionStorage.setItem(INVITE_OFFER, "1");
  } catch { /* Installation is always reachable from the rail. */ }
}

/** The first ordinary launch from the icon can point to device enrollment. */
export function takeInstallOffer() {
  try {
    if (window.location.search) return false;
    if (installationMode() === "installed") {
      sessionStorage.removeItem(INVITE_OFFER);
      const key = "stellarr.installed-welcome";
      if (localStorage.getItem(key) || !("Notification" in window) || Notification.permission !== "default") return false;
      localStorage.setItem(key, "1");
      return true;
    }
    if (!sessionStorage.getItem(INVITE_OFFER)) return false;
    sessionStorage.removeItem(INVITE_OFFER);
    if (localStorage.getItem(OFFER_SEEN)) return false;
    localStorage.setItem(OFFER_SEEN, "1");
    return true;
  } catch { return false; }
}
