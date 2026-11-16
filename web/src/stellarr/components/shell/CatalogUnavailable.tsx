import { ErrorNotice } from "@/components/primitives/ErrorNotice";
import type { SupportedLocale } from "@/i18n";
import { CATALOG_UNAVAILABLE } from "@/i18n/messages/unavailable";

/**
 * What the page shows in place of the app when its language's catalog cannot
 * be fetched. It reads no catalog, and loading the page again fetches the
 * current release's.
 */
export function CatalogUnavailable({ locale, onReload }: { locale: SupportedLocale; onReload: () => void }) {
  const text = CATALOG_UNAVAILABLE[locale];
  return (
    <main style={{ height: "100%", display: "grid", placeItems: "center", padding: 24 }}>
      <div style={{ width: "min(420px, 100%)" }}>
        <ErrorNotice
          title={text.title}
          message={text.message}
          icon="wifi"
          actions={[{ label: text.action, onClick: onReload }]}
        />
      </div>
    </main>
  );
}
