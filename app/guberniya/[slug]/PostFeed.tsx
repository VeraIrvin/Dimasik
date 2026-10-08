import type { ReactNode } from "react";
import Link from "next/link";
import type { GuberniaPost, PublishedOption, Settlement } from "@/lib/gubernia-publications";
import PostAdminControls from "./PostAdminControls";
import PostBody from "./PostBody";
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
  /** Keeps the title navigator visible when `posts` is empty (a filtered province feed). */
  keepSidebarWhenEmpty?: boolean;
  /** Left column heading; the right column keeps the posts themselves. */
  sidebarHeading?: string;
  /** Optional controls rendered before the heading in the sticky title navigator. */
  sidebarControls?: ReactNode;
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
  sidebarHeading = "Сообщения",
  sidebarControls,
}: Props) {
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
    <div className={sidebarControls ? `${styles.feed} ${styles.feedWithControls}` : styles.feed}>
      <aside
        className={sidebarControls ? `${styles.sidebar} ${styles.sidebarWithControls}` : styles.sidebar}
        aria-label="Содержание сообщений"
      >
        {sidebarControls ? <div className={styles.sidebarControls}>{sidebarControls}</div> : null}
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
            post.category || districtName || settlement || post.year || post.archiveReference;

          return (
            <article key={post.id} id={`post-${post.id}`} className={styles.post}>
              <h2 className={styles.postTitle}>{post.title}</h2>
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
                  {post.archiveReference ? <><dt>Архивный шифр</dt><dd>{post.archiveReference}</dd></> : null}
                </dl>
              ) : null}
              <PostBody body={post.body} />
            </article>
          );
        }) : <p className={styles.noPosts}>{emptyMessage}</p>}
      </section>
    </div>
  );
}
