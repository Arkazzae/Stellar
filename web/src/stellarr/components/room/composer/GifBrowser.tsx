import { useEffect, useId, useImperativeHandle, useRef, useState, type Ref, type RefObject } from "react";

import { useLayer } from "@stellarr/ui/behavior";
import { Icon } from "@stellarr/icons";
import type { SupportedLocale } from "@/i18n";
import {
  fetchGiphyPage,
  GIPHY_REFERRER_POLICY,
  resolveGiphyApiKey,
  trackGiphyAction,
  type GiphyGif,
} from "@/lib/giphy/giphy";
import { useLanguage } from "@/state/LanguageProvider";

const PAGE_SIZE = 24;
/** As long as a search may be. */
export const GIF_QUERY_MAX_LENGTH = 50;
/** A blank query is the trending list and loads at once; typing waits for a pause. */
const TYPING_PAUSE_MS = 280;
const SKELETON_TILES = 12;
/** One row of placeholders under the results while the next page comes. */
const MORE_SKELETON_TILES = 3;
/** How far below what is visible the next page already starts loading. */
const LOAD_AHEAD_PX = 320;

// GIPHY's official horizontal mark. It is inline so the attribution needs no
// request of its own.
export const GIPHY_MARK =
  "data:image/png;base64,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";

/**
 * The results for a query, a page at a time.
 *
 * Nothing loads while the panel is closed. Every new search bumps `run`, so a
 * response (or a "show more") that belongs to an earlier search is dropped
 * instead of overwriting the current one. A search is made in the interface's
 * language.
 */
function useGifResults(locale: SupportedLocale, query: string, active: boolean) {
  const [items, setItems] = useState<GiphyGif[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const run = useRef(0);

  useEffect(() => {
    if (!active) return;
    const current = ++run.current;
    const controller = new AbortController();
    const timer = window.setTimeout(
      () => {
        setLoading(true);
        // A "show more" still in flight belongs to the search being replaced.
        setLoadingMore(false);
        setFailed(false);
        resolveGiphyApiKey()
          .then((key) =>
            fetchGiphyPage({
              apiKey: key,
              locale,
              query,
              limit: PAGE_SIZE,
              signal: controller.signal,
            }),
          )
          .then((page) => {
            if (current !== run.current) return;
            setItems(page.items);
            setTotal(page.totalCount);
          })
          .catch(() => {
            if (controller.signal.aborted || current !== run.current) return;
            setItems([]);
            setFailed(true);
          })
          .finally(() => {
            if (current === run.current) setLoading(false);
          });
      },
      query.trim() === "" ? 0 : TYPING_PAUSE_MS,
    );
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [locale, active, query, attempt]);

  const loadMore = async () => {
    if (loadingMore || items.length >= total) return;
    const current = run.current;
    setLoadingMore(true);
    setFailed(false);
    try {
      const key = await resolveGiphyApiKey();
      const page = await fetchGiphyPage({
        apiKey: key,
        locale,
        query,
        offset: items.length,
        limit: PAGE_SIZE,
      });
      if (current !== run.current) return;
      setItems((shown) => {
        const known = new Set(shown.map((gif) => gif.id));
        return [...shown, ...page.items.filter((gif) => !known.has(gif.id))];
      });
      setTotal(page.totalCount);
    } catch {
      if (current === run.current) setFailed(true);
    } finally {
      if (current === run.current) setLoadingMore(false);
    }
  };

  return {
    items,
    hasMore: items.length < total,
    loading,
    loadingMore,
    failed,
    retry: () => setAttempt((count) => count + 1),
    loadMore,
  };
}

/**
 * Loads the next page as the end of the results comes near, so scrolling down
 * is all it takes. It is armed again whenever the list grows: an observer
 * only speaks on a change, and a page too short to push the end out of view
 * must still lead to the next one. The caller disarms it after a failed page,
 * so a dead connection is not asked again until the person says so.
 */
function useLoadOnReach(
  root: RefObject<HTMLElement | null>,
  end: RefObject<HTMLElement | null>,
  armed: boolean,
  shown: number,
  load: () => void,
) {
  // The observer outlives renders; it must call this render's loader, which
  // knows how many GIFs are already shown.
  const latest = useRef(load);
  useEffect(() => {
    latest.current = load;
  });

  useEffect(() => {
    const target = end.current;
    if (!armed || target === null || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) latest.current();
      },
      { root: root.current, rootMargin: `0px 0px ${LOAD_AHEAD_PX}px 0px` },
    );
    observer.observe(target);
    return () => observer.disconnect();
  }, [armed, shown, root, end]);
}

/** Opens the panel from elsewhere, such as the composer's /gif, with the words to look for. */
export interface GifBrowserHandle {
  open(query: string): void;
}

export interface GifBrowserProps {
  /** Sends the chosen GIF. A rejection keeps the browser open and shows an error. */
  onSelect(pageUrl: string): Promise<void>;
  ref?: Ref<GifBrowserHandle>;
}

/**
 * The GIF button and the panel it opens. Choosing a GIF sends it at once and
 * closes the panel; the panel stays mounted but hidden, so its search survives
 * a close. While open it is a layer on the shared stack: Escape and a press
 * elsewhere close it when no layer above it takes them.
 */
export function GifBrowser({ onSelect, ref }: GifBrowserProps) {
  const { t, locale } = useLanguage();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [sendingId, setSendingId] = useState<string | null>(null);
  const [sendFailed, setSendFailed] = useState(false);
  const results = useGifResults(locale, query, open);

  const panelId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const resultsRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  // GIPHY wants one view event per GIF, however often it scrolls into view.
  const viewed = useRef(new Set<string>());

  const close = () => {
    setOpen(false);
    triggerRef.current?.focus();
  };

  useImperativeHandle(
    ref,
    () => ({
      open(words: string) {
        setQuery(words.slice(0, GIF_QUERY_MAX_LENGTH));
        setSendFailed(false);
        setOpen(true);
      },
    }),
    [],
  );

  useEffect(() => {
    if (!open) return;
    const frame = requestAnimationFrame(() => searchRef.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [open]);

  // The panel stays mounted while closed, so it is a layer only while open. The
  // trigger toggles the panel itself; closing on a press there too would
  // reopen it.
  useLayer(open, {
    node: () => panelRef.current,
    owns: (target) =>
      panelRef.current?.contains(target) === true || triggerRef.current?.contains(target) === true,
    onEscape: () => {
      close();
    },
    onPointerDownOutside: () => {
      setOpen(false);
    },
  });

  const send = async (gif: GiphyGif) => {
    if (sendingId !== null) return;
    setSendingId(gif.id);
    setSendFailed(false);
    void trackGiphyAction(gif.analytics.onclick);
    try {
      await onSelect(gif.pageUrl);
      void trackGiphyAction(gif.analytics.onsent);
      close();
    } catch {
      setSendFailed(true);
    } finally {
      setSendingId(null);
    }
  };

  const markViewed = (gif: GiphyGif) => {
    if (viewed.current.has(gif.id)) return;
    viewed.current.add(gif.id);
    void trackGiphyAction(gif.analytics.onload);
  };

  const { items, loading, loadingMore, failed } = results;
  const term = query.trim();
  const busy = sendingId !== null;
  // Without an observer (an old browser) the next page waits for the button.
  const autoLoads = typeof IntersectionObserver !== "undefined";
  useLoadOnReach(
    resultsRef,
    endRef,
    open && autoLoads && !loading && !loadingMore && !failed && results.hasMore,
    items.length,
    () => void results.loadMore(),
  );

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="attach gif-trigger"
        aria-label={t("gif.browse")}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((was) => !was)}
      >
        GIF
      </button>
      <div
        ref={panelRef}
        id={panelId}
        className="gif-panel"
        role="dialog"
        aria-modal="false"
        aria-label={t("gif.browser")}
        hidden={!open}
      >
        <div className="gif-search-row">
          <label className="gif-search">
            <span className="sr-only">{t("gif.search")}</span>
            <Icon name="search" size={16} />
            <input
              ref={searchRef}
              type="search"
              value={query}
              maxLength={GIF_QUERY_MAX_LENGTH}
              placeholder={t("gif.searchPlaceholder")}
              disabled={busy}
              onChange={(event) => {
                setQuery(event.target.value);
                setSendFailed(false);
              }}
            />
          </label>
          <button type="button" className="gif-close" aria-label={t("gif.close")} onClick={close}>
            <Icon name="x" size={16} />
          </button>
        </div>

        <div className="gif-panel-head">
          <span>{term === "" ? t("gif.popular") : t("gif.resultsFor", { query: term })}</span>
          <span>{t("gif.sendImmediately")}</span>
        </div>

        <div ref={resultsRef} className="gif-results" aria-busy={loading || loadingMore}>
          {loading && (
            <div className="gif-grid" aria-label={t("gif.loading")}>
              {Array.from({ length: SKELETON_TILES }, (_, index) => (
                <span className="gif-skeleton" key={index} />
              ))}
            </div>
          )}
          {!loading && items.length === 0 && (
            failed ? (
              <div className="gif-state" role="alert">
                <p>{t("gif.fetchFailed")}</p>
                <button type="button" onClick={results.retry}>
                  {t("gif.retry")}
                </button>
              </div>
            ) : (
              <div className="gif-state">
                <p>{t("gif.noResults")}</p>
                <span>{t("gif.noResultsDetail")}</span>
              </div>
            )
          )}
          {!loading && items.length > 0 && (
            <>
              <div className="gif-grid">
                {items.map((gif) => (
                  <button
                    type="button"
                    className="gif-tile"
                    key={gif.id}
                    aria-label={t("gif.send", { alt: gif.alt ?? t("gif.untitled") })}
                    disabled={busy}
                    onClick={() => void send(gif)}
                  >
                    <picture>
                      {gif.stillUrl !== undefined && (
                        <source media="(prefers-reduced-motion: reduce)" srcSet={gif.stillUrl} />
                      )}
                      <img
                        src={gif.previewUrl}
                        alt=""
                        loading="lazy"
                        referrerPolicy={GIPHY_REFERRER_POLICY}
                        onLoad={() => markViewed(gif)}
                      />
                    </picture>
                    {gif.username !== undefined && (
                      <span className="gif-creator">@{gif.username}</span>
                    )}
                    {sendingId === gif.id && <span className="gif-sending">{t("gif.sending")}</span>}
                  </button>
                ))}
                {loadingMore &&
                  Array.from({ length: MORE_SKELETON_TILES }, (_, index) => (
                    <span className="gif-skeleton" key={`more-${index}`} aria-hidden="true" />
                  ))}
              </div>
              {results.hasMore && autoLoads && !failed && (
                // Where the next page is asked for, once it comes near.
                <div ref={endRef} className="gif-end" aria-hidden="true" />
              )}
              {results.hasMore && (!autoLoads || failed) && (
                <button
                  type="button"
                  className="gif-more"
                  disabled={loadingMore || busy}
                  onClick={() => void results.loadMore()}
                >
                  {loadingMore ? t("gif.loadingMore") : failed ? t("gif.retry") : t("gif.showMore")}
                </button>
              )}
            </>
          )}
        </div>

        {/* A failed "show more" leaves the earlier results on screen. */}
        {failed && items.length > 0 && (
          <p className="gif-send-error" role="alert">
            {t("gif.fetchFailed")}
          </p>
        )}
        {sendFailed && (
          <p className="gif-send-error" role="alert">
            {t("gif.sendFailed")}
          </p>
        )}
        <a
          className="giphy-attribution"
          href="https://giphy.com/"
          target="_blank"
          rel="noopener noreferrer"
        >
          <img src={GIPHY_MARK} alt={t("gif.poweredBy")} width={100} height={21} />
        </a>
      </div>
    </>
  );
}
