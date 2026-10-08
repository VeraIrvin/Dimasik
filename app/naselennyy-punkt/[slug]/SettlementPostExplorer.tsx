"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import PostFeed from "@/app/guberniya/[slug]/PostFeed";
import type { GuberniaPost, PublishedOption, Settlement } from "@/lib/gubernia-publications";
import styles from "./page.module.css";

type Props = {
  posts: GuberniaPost[];
  guberniaId: string;
  provinces: PublishedOption[];
  settlements: Settlement[];
  /** Category options configured in the admin settings. */
  categories: string[];
  districtNames: Record<string, string>;
  isAdmin: boolean;
  /** Contextual map destination, identical to the in-flow header back link. */
  backHref: string;
};

/**
 * Settlement posts with a year-only filter. The year filter and its reset live
 * in PostFeed's persistent left navigator, and the selected year feeds a single
 * array into PostFeed, so the title navigator and the articles never diverge.
 * The rail only takes over the «Вернуться к карте» link once the page header is
 * scrolled away, which keeps exactly one back control visible at any position.
 */
export default function SettlementPostExplorer({
  posts,
  guberniaId,
  provinces,
  settlements,
  categories,
  districtNames,
  isAdmin,
  backHref,
}: Props) {
  const [year, setYear] = useState("");
  const [headerScrolledAway, setHeaderScrolledAway] = useState(false);

  useEffect(() => {
    const update = () => {
      const header = document.getElementById("settlement-page-header");
      setHeaderScrolledAway(header ? header.getBoundingClientRect().bottom <= 0 : false);
    };

    update();
    window.addEventListener("scroll", update, { passive: true });
    window.addEventListener("resize", update);
    return () => {
      window.removeEventListener("scroll", update);
      window.removeEventListener("resize", update);
    };
  }, []);

  useEffect(() => {
    window.dispatchEvent(new Event("settlement-rail-controls-change"));
  }, [headerScrolledAway]);

  // Same ordering as the province filter; legacy posts without a year stay in «Все годы».
  const years = useMemo(
    () => Array.from(new Set(posts.map((post) => post.year).filter(Boolean))).sort(
      (left, right) => right.localeCompare(left, "ru", { numeric: true }),
    ),
    [posts],
  );
  // A refresh after create/edit/delete can retire the selected year; fall back to all posts.
  const activeYear = years.includes(year) ? year : "";
  const visiblePosts = useMemo(
    () => (activeYear ? posts.filter((post) => post.year === activeYear) : posts),
    [posts, activeYear],
  );

  const sidebarControls = (
    <div id="settlement-rail-controls" className={styles.railFilter}>
      <div className={styles.railPrimary}>
        {headerScrolledAway ? (
          <Link id="settlement-rail-back" className={styles.railBack} href={backHref}>
            ← Вернуться к карте
          </Link>
        ) : null}
        <select
          id="settlement-post-year"
          aria-label="Фильтр по году"
          value={activeYear}
          onChange={(event) => setYear(event.target.value)}
        >
          <option value="">Все годы</option>
          {years.map((option) => (
            <option key={option} value={option}>{option}</option>
          ))}
        </select>
      </div>
      <button
        className={styles.railReset}
        type="button"
        onClick={() => setYear("")}
        disabled={activeYear === ""}
      >
        Сбросить фильтр
      </button>
    </div>
  );

  return (
    <PostFeed
      posts={visiblePosts}
      guberniaId={guberniaId}
      provinces={provinces}
      settlements={settlements}
      categories={categories}
      districtNames={districtNames}
      isAdmin={isAdmin}
      ariaLabel="Сообщения населённого пункта"
      emptyMessage={
        activeYear
          ? "По выбранному году сообщений нет."
          : "Сообщений об этом населённом пункте пока нет."
      }
      sidebarHeading="Содержание"
      sidebarControls={sidebarControls}
      keepSidebarWhenEmpty={posts.length > 0}
    />
  );
}
