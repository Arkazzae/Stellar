import { Icon } from "@stellarr/icons";
import { formatFull } from "@/lib/format/format";
import { safeAssetUrl, safeHttpUrl } from "@/lib/format/safeUrl";
import { useLanguage } from "@/state/hooks";

import type { LinkItemVM } from "./channelPanelModel";
import { IndexedTab, type IndexStatus } from "./IndexedTab";

export function LinksTab({
  links,
  status,
}: {
  links: LinkItemVM[];
  status: IndexStatus;
}) {
  const { locale, t } = useLanguage();
  const safeLinks = links.flatMap((link) => {
    const href = safeHttpUrl(link.href);
    return href === undefined ? [] : [{ ...link, href, thumb: safeAssetUrl(link.thumb) }];
  });

  return (
    <IndexedTab
      status={status}
      empty={safeLinks.length === 0}
      emptyText={t("room.noLinksInChannel")}
    >
      <ul className="cp-list">
        {safeLinks.map((link) => (
          <li key={link.id}>
            <a
              className="cp-row"
              href={link.href}
              target="_blank"
              rel="ugc nofollow noopener noreferrer"
            >
              {link.thumb === undefined ? (
                <span className="cp-favicon" aria-hidden="true">
                  <Icon name="link" size={14} />
                </span>
              ) : (
                <img className="cp-thumb" src={link.thumb} alt="" loading="lazy" />
              )}
              <span className="cp-meta">
                <b>{link.title}</b>
                <span>{link.domain}</span>
                <span>
                  {link.authorName} · {formatFull(link.timestamp, locale)}
                </span>
              </span>
            </a>
          </li>
        ))}
      </ul>
    </IndexedTab>
  );
}
