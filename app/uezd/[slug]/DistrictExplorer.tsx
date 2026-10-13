"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import AdminCreationPanel from "@/app/guberniya/[slug]/AdminCreationPanel";
import PostFeed from "@/app/guberniya/[slug]/PostFeed";
import PostImageGallery from "@/app/guberniya/[slug]/PostImageGallery";
import ProvinceFilterControls from "@/app/guberniya/[slug]/ProvinceFilterControls";
import ProvinceQuickNavigation from "@/app/guberniya/[slug]/ProvinceQuickNavigation";
import RichDocumentPreview from "@/components/RichDocumentPreview";
import UyezdsMap from "@/components/UyezdsMap";
import type {
  PublishedDistrict,
  PublishedOption,
  Settlement,
} from "@/lib/gubernia-publications";
import styles from "./page.module.css";

type Props = {
  district: PublishedDistrict;
  provinces: PublishedOption[];
  editorSettlements: Settlement[];
  categories: string[];
  settlementTypes: string[];
  isAdmin: boolean;
  backHref: string;
  highlightQuery: string;
};


/**
 * District-local map and feed. The district itself is an immutable geographic
 * scope; the controls only filter its settlements, years, and categories.
 */
export default function DistrictExplorer({
  district,
  provinces,
  editorSettlements,
  categories,
  settlementTypes,
  isAdmin,
  backHref,
  highlightQuery,
}: Props) {
  const router = useRouter();
  const [settlementId, setSettlementId] = useState<string | null>(null);
  const [year, setYear] = useState("");
  const [category, setCategory] = useState("");
  const [referenceExpanded, setReferenceExpanded] = useState(false);

  useEffect(() => {
    if (!highlightQuery) return;

    function revealHashTarget() {
      if (!window.location.hash.startsWith("#post-")) return;
      setSettlementId(null);
      setYear("");
      setCategory("");
    }

    revealHashTarget();
    window.addEventListener("hashchange", revealHashTarget);
    window.addEventListener("popstate", revealHashTarget);
    return () => {
      window.removeEventListener("hashchange", revealHashTarget);
      window.removeEventListener("popstate", revealHashTarget);
    };
  }, [highlightQuery]);

  const districtSettlements = useMemo(
    () => [...district.settlements].sort((left, right) => left.name.localeCompare(right.name, "ru")),
    [district.settlements],
  );
  const settlementById = useMemo(
    () => new Map(districtSettlements.map((settlement) => [settlement.id, settlement])),
    [districtSettlements],
  );
  const activeSettlementId = settlementId && settlementById.has(settlementId)
    ? settlementId
    : null;
  const years = useMemo(
    () => Array.from(new Set(district.posts.map((post) => post.year).filter(Boolean))).sort(
      (left, right) => right.localeCompare(left, "ru", { numeric: true }),
    ),
    [district.posts],
  );
  const categoryOptions = useMemo(
    () => [
      ...categories,
      ...Array.from(
        new Set(
          district.posts
            .map((post) => post.category)
            .filter((value): value is string => Boolean(value)),
        ),
      )
        .filter((option) => !categories.includes(option))
        .sort((left, right) => left.localeCompare(right, "ru")),
    ],
    [categories, district.posts],
  );
  const districtNames = useMemo(
    () => ({ [district.id]: district.name }),
    [district.id, district.name],
  );
  const activeYear = years.includes(year) ? year : "";
  const activeCategory = categoryOptions.includes(category) ? category : "";
  const visiblePosts = useMemo(
    () => district.posts.filter(
      (post) =>
        (!activeSettlementId || post.settlementId === activeSettlementId) &&
        (!activeYear || post.year === activeYear) &&
        (!activeCategory || post.category === activeCategory),
    ),
    [activeCategory, activeSettlementId, activeYear, district.posts],
  );

  const openSettlement = useCallback((nextSettlementId: string) => {
    const settlement = settlementById.get(nextSettlementId);
    if (settlement) router.push(`${settlement.url}?from=district`);
  }, [router, settlementById]);

  function selectSettlement(nextSettlementId: string | null) {
    setSettlementId(nextSettlementId === activeSettlementId ? null : nextSettlementId);
  }

  function resetFilters() {
    setSettlementId(null);
    setYear("");
    setCategory("");
  }

  const hasFilters = activeSettlementId !== null || activeYear !== "" || activeCategory !== "";
  const filterControlsProps = {
    settlements: districtSettlements,
    years,
    categories: categoryOptions,
    settlementId: activeSettlementId,
    year: activeYear,
    category: activeCategory,
    hasFilters,
    onSettlementChange: selectSettlement,
    onYearChange: setYear,
    onCategoryChange: setCategory,
    onReset: resetFilters,
  };

  return (
    <>
      <ProvinceQuickNavigation
        filtersId="district-post-filters"
        upTargetId="district-map"
        backHref={backHref}
        navigationLabel="Навигация по странице уезда"
        upLabel="К карте уезда"
      >
        <ProvinceFilterControls idPrefix="district-toolbar" compact {...filterControlsProps} />
      </ProvinceQuickNavigation>

      {district.description || district.images.length > 0 ? (
        <section className={styles.reference} aria-labelledby="district-reference-heading">
          <h2 className={styles.referenceHeading} id="district-reference-heading">
            Справочные сведения
          </h2>
          {district.images.length > 0 ? (
            <PostImageGallery images={district.images} ariaLabel="Изображения уезда" />
          ) : null}
          {district.description ? (
            <RichDocumentPreview
              body={district.description}
              expanded={referenceExpanded}
              onExpandedChange={setReferenceExpanded}
            />
          ) : null}
        </section>
      ) : null}

      <div className={styles.districtMap} id="district-map">
        <UyezdsMap
          guberniaId={district.guberniaId}
          guberniaName={district.gubernia.name}
          settlements={districtSettlements}
          districtNames={districtNames}
          scopeDistrictId={district.id}
          selectedDistrictId={null}
          selectedSettlementId={activeSettlementId}
          onDistrictSelect={() => setSettlementId(null)}
          onSettlementOpen={openSettlement}
        />
      </div>

      {isAdmin ? (
        <div className={styles.composer}>
          <AdminCreationPanel
            guberniaId={district.guberniaId}
            provinces={provinces}
            districts={district.gubernia.districts}
            settlements={editorSettlements}
            categories={categories}
            settlementTypes={settlementTypes}
            defaultUyezdId={district.id}
          />
        </div>
      ) : null}

      <section
        className={styles.filters}
        id="district-post-filters"
        aria-label="Фильтры сообщений"
      >
        <ProvinceFilterControls idPrefix="district-post" {...filterControlsProps} />
      </section>

      <PostFeed
        posts={visiblePosts}
        guberniaId={district.guberniaId}
        provinces={provinces}
        districts={district.gubernia.districts}
        settlements={editorSettlements}
        categories={categories}
        districtNames={districtNames}
        isAdmin={isAdmin}
        ariaLabel="Сообщения уезда"
        emptyMessage={
          district.posts.length > 0
            ? "По выбранным фильтрам сообщений нет."
            : "Сообщений об этом уезде пока нет."
        }
        keepSidebarWhenEmpty={district.posts.length > 0}
        highlightQuery={highlightQuery}
      />
      <span className={styles.visuallyHidden} role="status">
        {hasFilters ? `Показано сообщений: ${visiblePosts.length}.` : ""}
      </span>
    </>
  );
}
