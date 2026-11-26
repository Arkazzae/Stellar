import { ErrorNotice } from "@/components/primitives/ErrorNotice";
import { locales, type SupportedLocale } from "@/i18n";

/**
 * What the page shows in place of the app when its language's catalog cannot
 * be fetched. It reads no catalog, and loading the page again fetches the
 * current release's.
 */
export function CatalogUnavailable({ locale, onReload }: { locale: SupportedLocale; onReload: () => void }) {
  const text = locales[locale].unavailable;
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
