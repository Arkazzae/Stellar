/** Android derives its launcher marker from visible notifications. Other
 * notifications (for example, a call) keep their own lifetime. */
export async function dismissActivityNotifications() {
  const registration = await navigator.serviceWorker?.getRegistration?.();
  const notifications = await registration?.getNotifications?.();
  for (const notification of notifications ?? []) {
    if (notification.data?.notificationId || notification.tag === "stellarr-activity") notification.close();
  }
}
