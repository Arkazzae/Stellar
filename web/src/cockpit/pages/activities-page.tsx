import { Button, Card, Input, Label } from "@stellarr/ui";
import { useState } from "react";

import { PageHeader } from "../components/page-header";
import { t } from "../i18n";
import { CockpitApiError, cockpitRequest } from "../lib/api";
import { useCockpitResource } from "../lib/use-cockpit-resource";

interface Manifest {
  id: string; name: string; version: string; description: string; permissions: string[];
  connects: { host: string; why: string }[];
}
interface Artwork { icon: string | null; cover: string | null }
interface Review { artwork?: Artwork; source: string; release: string; revision: string; digest: string; bytes: number; manifest: Manifest }
interface Installed extends Review { install: string; enabled: boolean }
interface Choice { release: string; id: string; name: string; version: string; bytes: number }
interface Selection { source: string; entries: Choice[] }
const base = "/api/cockpit/activities";
const library = (signal?: AbortSignal) => cockpitRequest<{ activities: Installed[]; hosting: boolean }>(base, {}, signal);
const post = <T,>(path: string, body: unknown) => cockpitRequest<T>(base + "/" + path, { method: "POST", body });

function permission(name: string) {
  if (name === "media.microphoneLevel") return t("activities.mic");
  if (name === "media.cameraTile") return t("activities.camera");
  return name;
}

function ActivityArtwork({ artwork, name }: { artwork?: Artwork; name: string }) {
  return <div aria-hidden="true" className="relative h-28 overflow-hidden rounded-md bg-muted">
    {artwork?.cover && <img src={artwork.cover} alt="" loading="lazy" className="h-full w-full object-cover" />}
    <div className="absolute bottom-3 left-3 flex size-14 items-center justify-center overflow-hidden rounded-md border bg-background text-xl font-semibold text-foreground">
      {artwork?.icon ? <img src={artwork.icon} alt="" className="h-full w-full object-cover" /> : name.slice(0, 1)}
    </div>
  </div>;
}

export function ActivitiesPage() {
  const { data, error, refresh } = useCockpitResource(library, t("activities.failed"));
  const [url, setUrl] = useState("");
  const [selection, setSelection] = useState<Selection | null>(null);
  const [review, setReview] = useState<Review | null>(null);
  const [update, setUpdate] = useState<Installed | null>(null);
  const [uninstall, setUninstall] = useState<Installed | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const size = (bytes: number) => new Intl.NumberFormat(document.documentElement.lang, {style:"unit",unit:"megabyte",maximumFractionDigits:2}).format(bytes / 1_000_000);
  async function run(fn: () => Promise<void>) {
    setBusy(true); setMessage("");
    try { await fn(); }
    catch (cause) {
      setMessage(cause instanceof CockpitApiError ? cause.errors.activity?.[0] ?? cause.message :
        cause instanceof Error ? cause.message : t("activities.failed"));
    } finally { setBusy(false); }
  }
  async function inspect(release: string, id: string, existing?: Installed) {
    setReview(null); setUpdate(existing ?? null);
    const result = await post<Review | { current: true }>("review", { url: release, id, update: existing?.install });
    if ("current" in result) setMessage(t("activities.current")); else setReview(result);
  }
  async function discover() {
    setSelection(null); setReview(null); setUpdate(null);
    const result = await post<Selection>("discover", { url });
    const first = result.entries[0];
    if (first && result.entries.length === 1) await inspect(first.release, first.id);
    else setSelection(result);
  }
  async function manage(entry: Installed, action: "enable" | "disable" | "uninstall") {
    await post(entry.install + "/manage", {action});
    setUninstall(null); setReview(null); await refresh();
  }
  return <div className="space-y-6">
    <PageHeader title={t("nav.activities")} description={t("activities.intro")} />
    <Card className="space-y-3 p-5">
      <form className="flex flex-wrap items-end gap-3" onSubmit={event => { event.preventDefault(); void run(discover); }}>
        <div className="min-w-60 flex-1 space-y-2"><Label htmlFor="activity-url">{t("activities.url")}</Label>
          <Input id="activity-url" required type="url" value={url} onChange={e => setUrl(e.target.value)} placeholder="https://github.com/owner/repository" /></div>
        <Button disabled={busy} type="submit">{t("activities.add")}</Button>
      </form>
      {selection && <div className="space-y-2"><h2>{t("activities.choose")}</h2>{selection.entries.map(entry =>
        <Button key={entry.id} disabled={busy} variant="secondary" onClick={() => void run(() => inspect(entry.release, entry.id))}>
          {entry.name} · {entry.version} · {size(entry.bytes)}
        </Button>)}</div>}
    </Card>
    {review && <Card className="space-y-4 p-5">
      <ActivityArtwork artwork={review.artwork} name={review.manifest.name} />
      <div><h2 className="text-xl font-semibold">{review.manifest.name} · {update ? update.manifest.version + " → " : ""}{review.manifest.version}</h2>
        <p>{review.manifest.description}</p>
        <a href={review.release} target="_blank" rel="noreferrer">{review.release}</a> · {size(review.bytes)}</div>
      <div><h3 className="font-semibold">{t("activities.permissions")}</h3>
        {review.manifest.permissions.length ? <ul>{review.manifest.permissions.map(name => <li key={name}>
          {permission(name)} {update && !update.manifest.permissions.includes(name) && <strong>{t("activities.added")}</strong>}</li>)}</ul> : t("activities.none")}
        {update?.manifest.permissions.filter(name => !review.manifest.permissions.includes(name)).map(name =>
          <p key={name}>{permission(name)} · {t("activities.removed")}</p>)}</div>
      <div><h3 className="font-semibold">{t("activities.hosts")}</h3>
        {review.manifest.connects.length ? <ul>{review.manifest.connects.map(({host,why}) => <li key={host}>
          <strong>{host}</strong> · {why} {update && !update.manifest.connects.some(old => old.host === host && old.why === why) && <b>{t("activities.changed")}</b>}
        </li>)}</ul> : t("activities.none")}
        {update?.manifest.connects.filter(old => !review.manifest.connects.some(next => next.host === old.host)).map(old =>
          <p key={old.host}>{old.host} · {t("activities.removed")}</p>)}</div>
      <div className="flex gap-2"><Button disabled={busy} onClick={() => void run(async () => {
        await post("install", { url: review.release, id: review.manifest.id, digest: review.digest, revision: review.revision, update: update?.install });
        setReview(null); setSelection(null); await refresh();
      })}>{update ? t("activities.update") : t("activities.install")}</Button>
        <Button variant="secondary" disabled={busy} onClick={() => setReview(null)}>{t("activities.cancel")}</Button></div>
    </Card>}
    {(error || message) && <p role="status">{message || error}</p>}
    {data && !data.hosting && <p>{t("activities.hosting")}</p>}
    {data?.activities.map(entry => <Card key={entry.install} className="space-y-3 p-5">
      <ActivityArtwork artwork={entry.artwork} name={entry.manifest.name} />
      <h2 className="text-xl font-semibold">{entry.manifest.name} · {entry.manifest.version}</h2>
      <p>{entry.manifest.description}</p>
      <p>{entry.enabled ? t("activities.enabled") : t("activities.disabled")} · <a href={entry.release} target="_blank" rel="noreferrer">{entry.source}</a></p>
      <div className="flex flex-wrap gap-2"><Button disabled={busy} variant="secondary" onClick={() => void run(() => manage(entry, entry.enabled ? "disable" : "enable"))}>
        {entry.enabled ? t("activities.disable") : t("activities.enable")}</Button>
        <Button disabled={busy} variant="secondary" onClick={() => void run(() => inspect(entry.source, entry.manifest.id, entry))}>{t("activities.check")}</Button>
        <Button disabled={busy} variant="secondary" onClick={() => setUninstall(entry)}>{t("activities.uninstall")}</Button></div>
      {uninstall?.install === entry.install && <div><p>{t("activities.confirmRemove")}</p>
        <Button disabled={busy} onClick={() => void run(() => manage(entry,"uninstall"))}>{t("activities.uninstall")}</Button>
        <Button variant="secondary" onClick={() => setUninstall(null)}>{t("activities.cancel")}</Button></div>}
    </Card>)}
  </div>;
}
