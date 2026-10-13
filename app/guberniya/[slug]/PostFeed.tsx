"use client";

import Link from "next/link";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import type { GuberniaPost, PublishedOption, Settlement } from "@/lib/gubernia-publications";
import {
  findTextHighlightRanges,
  normalizePostHighlightQuery,
} from "@/lib/post-search-highlight";
import PostAdminControls from "./PostAdminControls";
import PostBody from "./PostBody";
import PostImageGallery from "./PostImageGallery";
import styles from "./PostFeed.module.css";

type Props = {
  posts: GuberniaPost[];
  guberniaId: string;
  provinces: PublishedOption[];
  settlements: Settlement[];
  /** Category options configured in the admin settings. */
  categories: string[];
  /** District id → name; a post's district row is omitted when its id is missing. */
  districtNames: Record<string, string>;
  isAdmin: boolean;
  ariaLabel: string;
  emptyMessage: string;
  /** Keeps the title navigator visible when `posts` is empty (a filtered feed). */
  keepSidebarWhenEmpty?: boolean;
  /** Table-of-contents heading shown above the post links. */
  sidebarHeading?: string;
  /** Search phrase highlighted only in the hash-targeted article. */
  highlightQuery?: string;
};

function postIdFromHash(): string | null {
  if (!window.location.hash.startsWith("#post-")) return null;
  try {
    return decodeURIComponent(window.location.hash.slice("#post-".length));
  } catch {
    return null;
  }
}

function highlightedTitle(title: string, query: string): ReactNode {
  const ranges = findTextHighlightRanges(title, query);
  if (!ranges.length) return title;

  const parts: ReactNode[] = [];
  let offset = 0;
  for (const range of ranges) {
    if (range.start > offset) parts.push(title.slice(offset, range.start));
    parts.push(
      <mark
        key={`${range.start}-${range.end}`}
        className={styles.highlight}
        data-post-search-highlight=""
      >
        {title.slice(range.start, range.end)}
      </mark>,
    );
    offset = range.end;
  }
  if (offset < title.length) parts.push(title.slice(offset));
  return parts;
}

type ActiveHighlight = {
  query: string;
  targetId: string;
};

/** Shared two-column province/settlement feed: title navigator plus full post articles. */
export default function PostFeed({
  posts,
  guberniaId,
  provinces,
  settlements,
  categories,
  districtNames,
  isAdmin,
  ariaLabel,
  emptyMessage,
  keepSidebarWhenEmpty = false,
  sidebarHeading = "Содержание",
  highlightQuery = "",
}: Props) {
  const normalizedHighlightQuery = normalizePostHighlightQuery(highlightQuery);
  const availablePostIds = useMemo(() => new Set(posts.map((post) => post.id)), [posts]);
  const [activeHighlight, setActiveHighlight] = useState<ActiveHighlight | null>(null);

  useEffect(() => {
    let waitingForVisibility = false;

    function activateFromLocation() {
      const targetId = postIdFromHash();
      if (!normalizedHighlightQuery || !targetId || !availablePostIds.has(targetId)) {
        setActiveHighlight(null);
        return;
      }

      if (document.visibilityState !== "visible") {
        waitingForVisibility = true;
        return;
      }

      waitingForVisibility = false;
      setActiveHighlight((current) =>
        current?.query === normalizedHighlightQuery && current.targetId === targetId
          ? current
          : { query: normalizedHighlightQuery, targetId },
      );
    }

    function activateWhenVisible() {
      if (waitingForVisibility && document.visibilityState === "visible") activateFromLocation();
    }

    activateFromLocation();
    window.addEventListener("hashchange", activateFromLocation);
    window.addEventListener("popstate", activateFromLocation);
    document.addEventListener("visibilitychange", activateWhenVisible);
    return () => {
      window.removeEventListener("hashchange", activateFromLocation);
      window.removeEventListener("popstate", activateFromLocation);
      document.removeEventListener("visibilitychange", activateWhenVisible);
    };
  }, [availablePostIds, normalizedHighlightQuery]);

  useEffect(() => {
    if (!activeHighlight) return;
    const timeout = window.setTimeout(() => setActiveHighlight(null), 12_000);
    return () => window.clearTimeout(timeout);
  }, [activeHighlight]);

  useEffect(() => {
    if (!activeHighlight) return;
    let secondFrame = 0;
    const firstFrame = window.requestAnimationFrame(() => {
      secondFrame = window.requestAnimationFrame(() => {
        const article = document.getElementById(`post-${activeHighlight.targetId}`);
        const firstMark = article?.querySelector<HTMLElement>("[data-post-search-highlight]");
        (firstMark ?? article)?.scrollIntoView({ block: "center", behavior: "auto" });
      });
    });
    return () => {
      window.cancelAnimationFrame(firstFrame);
      if (secondFrame) window.cancelAnimationFrame(secondFrame);
    };
  }, [activeHighlight]);

  const settlementById: Record<string, Settlement> = Object.fromEntries(
    settlements.map((settlement): [string, Settlement] => [settlement.id, settlement]),
  );

  if (posts.length === 0 && !keepSidebarWhenEmpty) {
    return (
      <section className={styles.posts} aria-label={ariaLabel}>
        <p className={styles.noPosts}>{emptyMessage}</p>
      </section>
    );
  }

  return (
    <div className={styles.feed}>
      <aside className={styles.sidebar} aria-label="Содержание сообщений">
        <h2 className={styles.sidebarHeading}>{sidebarHeading}</h2>
        <nav className={styles.sidebarNav} aria-label="Перейти к сообщению">
          <ol className={styles.postLinks}>
            {posts.map((post) => (
              <li key={post.id}>
                <a href={`#post-${post.id}`}>{post.title}</a>
              </li>
            ))}
          </ol>
        </nav>
      </aside>

      <section className={styles.posts} aria-label={ariaLabel}>
        {posts.length ? posts.map((post) => {
          const settlement = post.settlementId ? settlementById[post.settlementId] : null;
          const districtName = post.uyezdId ? districtNames[post.uyezdId] : null;
          const hasMetadata =
            post.category || districtName || settlement || post.year || post.author || post.archiveReference;
          const postHighlightQuery =
            activeHighlight?.targetId === post.id ? activeHighlight.query : "";

          return (
            <article key={post.id} id={`post-${post.id}`} className={styles.post}>
              <h2 className={styles.postTitle}>
                {highlightedTitle(post.title, postHighlightQuery)}
              </h2>
              {isAdmin ? (
                <PostAdminControls
                  guberniaId={guberniaId}
                  post={post}
                  provinces={provinces}
                  settlements={settlements}
                  categories={categories}
                />
              ) : null}
              {hasMetadata ? (
                <dl className={styles.postMetadata}>
                  {post.category ? <><dt>Категория</dt><dd>{post.category}</dd></> : null}
                  {districtName ? <><dt>Уезд</dt><dd>{districtName}</dd></> : null}
                  {settlement ? (
                    <>
                      <dt>Населённый пункт</dt>
                      <dd>
                        <Link className={styles.settlementLink} href={settlement.url}>
                          {settlement.name}
                        </Link>
                      </dd>
                    </>
                  ) : null}
                  {post.year ? <><dt>Год</dt><dd>{post.year}</dd></> : null}
                  {post.author ? <><dt>Автор</dt><dd>{post.author}</dd></> : null}
                  {post.archiveReference ? <><dt>Источник</dt><dd>{post.archiveReference}</dd></> : null}
                </dl>
              ) : null}
              <PostImageGallery images={post.images} />
              <PostBody body={post.body} highlightQuery={postHighlightQuery} />
            </article>
          );
        }) : <p className={styles.noPosts}>{emptyMessage}</p>}
      </section>
    </div>
  );
}
