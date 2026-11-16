import { Link } from "@stellarr/router";
import { Button } from "@stellarr/ui";

import { useLanguage } from "@/state/hooks";

/**
 * Built from the same `.sv-h` heading and primary `Button` the settings
 * screens use. A 404 that looked like a different product would read as a
 * broken deployment rather than a wrong address.
 *
 * It answers every address no route claims, outside /cockpit, and speaks the
 * language the provider holds, which follows the person's own choice.
 */
export default function NotFoundPage() {
  const { t } = useLanguage();

  return (
    <main style={{ height: "100%", display: "grid", placeItems: "center", padding: 24 }}>
      <div style={{ width: "min(460px, 100%)" }}>
        <h1 className="sv-h" style={{ paddingRight: 0 }}>
          {t("notFound.title")}
          <span>{t("notFound.detail")}</span>
        </h1>
        <Button asChild variant="primary" className="nf-home">
          <Link href="/">{t("notFound.back")}</Link>
        </Button>
      </div>
    </main>
  );
}
