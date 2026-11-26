import { Button, Card, Input, Label, Spinner } from "@stellarr/ui";
import { useState } from "react";

import { t } from "../i18n";
import { CLIENT_KIND, CLIENT_VERSION } from "../lib/api";
import { CardTitleRow } from "./card-title-row";

type PairingCode = { code: string; instance: string };

export function ActivityPreviewPairing() {
  const [pairing, setPairing] = useState<PairingCode | null>(null);
  const [issuing, setIssuing] = useState(false);
  const [error, setError] = useState(false);

  async function generate() {
    setIssuing(true);
    setError(false);
    setPairing(null);
    try {
      const response = await fetch("/_stellarr/dev/pair-code", {
        method: "POST",
        credentials: "same-origin",
        headers: { "X-Stellarr-Client": CLIENT_KIND, "X-Stellarr-Client-Version": CLIENT_VERSION },
      });
      if (!response.ok) throw new Error("Pairing request failed.");
      const value = (await response.json()) as Partial<PairingCode>;
      if (typeof value.code !== "string" || typeof value.instance !== "string")
        throw new Error("Invalid pairing code.");
      setPairing({ code: value.code, instance: value.instance });
    } catch {
      setError(true);
    } finally {
      setIssuing(false);
    }
  }

  return (
    <Card className="gap-0 py-0">
      <CardTitleRow icon="login" title={t("integrations.activityPreview.title")} />
      <div className="space-y-4 px-5 py-5">
        <p className="text-sm text-muted-foreground">{t("integrations.activityPreview.description")}</p>
        {pairing ? (
          <div className="space-y-3">
            <div className="space-y-2">
              <Label htmlFor="activity-preview-pairing-code">{t("integrations.activityPreview.code")}</Label>
              <Input
                id="activity-preview-pairing-code"
                className="font-mono"
                readOnly
                value={pairing.code}
                onFocus={(event) => event.currentTarget.select()}
                aria-describedby="activity-preview-code-expiry"
              />
              <p id="activity-preview-code-expiry" className="text-xs text-muted-foreground">
                {t("integrations.activityPreview.expiry")}
              </p>
            </div>
            <p className="text-sm text-muted-foreground">{t("integrations.activityPreview.instruction")}</p>
            <code className="block overflow-x-auto rounded-md bg-muted px-3 py-2 text-xs">
              npx stellarr dev --instance {pairing.instance}
            </code>
          </div>
        ) : null}
        {error ? <p role="alert" className="text-sm text-danger">{t("integrations.activityPreview.failed")}</p> : null}
        <Button type="button" variant="secondary" disabled={issuing} onClick={() => void generate()}>
          {issuing ? <Spinner /> : null}
          {t(pairing ? "integrations.activityPreview.anotherCode" : "integrations.activityPreview.generateCode")}
        </Button>
      </div>
    </Card>
  );
}
