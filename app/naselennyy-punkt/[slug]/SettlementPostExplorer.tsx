"use client";

import { useMemo, useState } from "react";
import PostFeed from "@/app/guberniya/[slug]/PostFeed";
import ProvinceFilterControls from "@/app/guberniya/[slug]/ProvinceFilterControls";
import ProvinceQuickNavigation from "@/app/guberniya/[slug]/ProvinceQuickNavigation";
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
 * Settlement posts with year and category filters, like the province feed: one
 * filter state feeds the in-flow form, the fixed compact toolbar and a single
 * already-filtered array into PostFeed, so the title navigator and the articles
 * never diverge. The toolbar takes over the «Вернуться к карте» link once the
 * page header scrolls away, keeping exactly one back control visible at any
 * scroll position. The ↑ button watches the reference section's top edge: the
 * reference can be much taller than the viewport, so waiting for its bottom
 * would hide the button while the reference is still on screen.
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
  const [category, setCategory] = useState("");

  // Same ordering as the province filter; legacy posts without a year stay in «Все годы».
  const years = useMemo(
    () => Array.from(new Set(posts.map((post) => post.year).filter(Boolean))).sort(
      (left, right) => right.localeCompare(left, "ru", { numeric: true }),
    ),
    [posts],
  );
  // Configured categories keep their settings order; categories kept only by
  // older posts stay selectable so every stored post remains filterable.
  const categoryOptions = useMemo(
    () => [
      ...categories,
      ...Array.from(
        new Set(
          posts
            .map((post) => post.category)
            .filter((value): value is string => Boolean(value)),
        ),
      )
        .filter((option) => !categories.includes(option))
        .sort((left, right) => left.localeCompare(right, "ru")),
    ],
    [categories, posts],
  );
  // A refresh after create/edit/delete can retire the selected values; fall back to all.
  const activeYear = years.includes(year) ? year : "";
  const activeCategory = categoryOptions.includes(category) ? category : "";
  const visiblePosts = useMemo(
    () =>
      posts.filter(
        (post) =>
          (!activeYear || post.year === activeYear) &&
          (!activeCategory || post.category === activeCategory),
      ),
    [posts, activeYear, activeCategory],
  );
  const hasFilters = activeYear !== "" || activeCategory !== "";

  function resetFilters() {
    setYear("");
    setCategory("");
  }

  const filterControlsProps = {
    years,
    categories: categoryOptions,
    year: activeYear,
    category: activeCategory,
    hasFilters,
    onYearChange: setYear,
    onCategoryChange: setCategory,
    onReset: resetFilters,
  };

  return (
    <>
      <ProvinceQuickNavigation
        filtersId="settlement-post-filters"
        upTargetId="settlement-reference"
        upVisibilityEdge="top"
        backHref={backHref}
        navigationLabel="Навигация по странице населённого пункта"
        upLabel="К справочным сведениям"
      >
        <ProvinceFilterControls idPrefix="settlement-toolbar" compact {...filterControlsProps} />
      </ProvinceQuickNavigation>

      <section
        className={styles.filters}
        id="settlement-post-filters"
        aria-label="Фильтры сообщений"
      >
        <ProvinceFilterControls idPrefix="settlement-post" {...filterControlsProps} />
      </section>

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
          posts.length > 0
            ? "По выбранным фильтрам сообщений нет."
            : "Сообщений об этом населённом пункте пока нет."
        }
        keepSidebarWhenEmpty={posts.length > 0}
      />
    </>
  );
}
