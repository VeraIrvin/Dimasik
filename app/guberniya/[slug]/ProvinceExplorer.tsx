"use client";

import { useCallback, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import type { PublishedGubernia, PublishedOption, Settlement } from "@/lib/gubernia-publications";
import UyezdsMap from "@/components/UyezdsMap";
import RichDocumentPreview from "@/components/RichDocumentPreview";
import AdminCreationPanel from "./AdminCreationPanel";
import PostFeed from "./PostFeed";
import PostImageGallery from "./PostImageGallery";
import ProvinceFilterControls, { type DistrictOption } from "./ProvinceFilterControls";
import ProvinceQuickNavigation from "./ProvinceQuickNavigation";
import styles from "./page.module.css";

type Props = {
  gubernia: PublishedGubernia;
  provinces: PublishedOption[];
  settlements: Settlement[];
  categories: string[];
  settlementTypes: string[];
  isAdmin: boolean;
};

export default function ProvinceExplorer({
  gubernia,
  provinces,
  settlements,
  categories,
  settlementTypes,
  isAdmin,
}: Props) {
  const router = useRouter();
  const [districts, setDistricts] = useState<DistrictOption[]>([]);
  const [districtId, setDistrictId] = useState<string | null>(null);
  const [settlementId, setSettlementId] = useState<string | null>(null);
  const [year, setYear] = useState("");
  const [category, setCategory] = useState("");
  const [referenceExpanded, setReferenceExpanded] = useState(false);

  const provinceSettlements = useMemo(
    () => settlements.filter((settlement) => settlement.guberniaId === gubernia.id),
    [gubernia.id, settlements],
  );
  const settlementById = useMemo(
    () => new Map(provinceSettlements.map((settlement) => [settlement.id, settlement])),
    [provinceSettlements],
  );
  const districtNames = useMemo(
    () =>
      Object.fromEntries(
        districts.map((district): [string, string] => [district.id, district.name]),
      ),
    [districts],
  );
  const availableSettlements = useMemo(
    () => provinceSettlements.filter((settlement) => !districtId || settlement.uyezdId === districtId),
    [provinceSettlements, districtId],
  );
  const years = useMemo(
    () => Array.from(new Set(gubernia.posts.map((post) => post.year).filter(Boolean))).sort(
      (left, right) => right.localeCompare(left, "ru", { numeric: true }),
    ),
    [gubernia.posts],
  );
  // Configured categories keep their settings order; categories kept only by
  // older posts stay selectable so every stored post remains filterable.
  const categoryOptions = useMemo(
    () => [
      ...categories,
      ...Array.from(
        new Set(
          gubernia.posts
            .map((post) => post.category)
            .filter((value): value is string => Boolean(value)),
        ),
      )
        .filter((option) => !categories.includes(option))
        .sort((left, right) => left.localeCompare(right, "ru")),
    ],
    [categories, gubernia.posts],
  );
  const visiblePosts = useMemo(
    () => gubernia.posts.filter((post) =>
      (!districtId || post.uyezdId === districtId) &&
      (!settlementId || post.settlementId === settlementId) &&
      (!year || post.year === year) &&
      (!category || post.category === category),
    ),
    [gubernia.posts, districtId, settlementId, year, category],
  );

  const selectDistrict = useCallback((nextDistrictId: string | null) => {
    const selectedId = nextDistrictId !== null && nextDistrictId === districtId
      ? null
      : nextDistrictId;
    setDistrictId(selectedId);
    setSettlementId((previous) =>
      previous && selectedId && settlementById.get(previous)?.uyezdId === selectedId
        ? previous
        : null,
    );
  }, [districtId, settlementById]);

  const selectSettlement = useCallback((nextSettlementId: string | null) => {
    const settlement = nextSettlementId && nextSettlementId !== settlementId
      ? settlementById.get(nextSettlementId)
      : null;
    setSettlementId(settlement?.id ?? null);
    if (settlement) setDistrictId(settlement.uyezdId);
  }, [settlementById, settlementId]);

  // Map marker clicks open the settlement page instead of filtering the feed. The
  // destination is the address stored with the published settlement, so legacy entries
  // keep their id-based address; unknown ids simply do nothing.
  const openSettlement = useCallback((nextSettlementId: string) => {
    const settlement = settlementById.get(nextSettlementId);
    if (settlement) router.push(`${settlement.url}?from=province`);
  }, [router, settlementById]);

  const updateDistricts = useCallback((options: DistrictOption[]) => {
    setDistricts(options);
  }, []);

  function resetFilters() {
    setDistrictId(null);
    setSettlementId(null);
    setYear("");
    setCategory("");
  }

  const hasFilters = districtId !== null || settlementId !== null || year !== "" || category !== "";

  const filterControlsProps = {
    districts,
    settlements: availableSettlements,
    years,
    categories: categoryOptions,
    districtId,
    settlementId,
    year,
    category,
    hasFilters,
    onDistrictChange: selectDistrict,
    onSettlementChange: selectSettlement,
    onYearChange: setYear,
    onCategoryChange: setCategory,
    onReset: resetFilters,
  };

  return (
    <>
      <ProvinceQuickNavigation>
        <ProvinceFilterControls idPrefix="province-toolbar" compact {...filterControlsProps} />
      </ProvinceQuickNavigation>

      <div className={styles.provinceMap} id="province-map">
        <UyezdsMap
          guberniaId={gubernia.id}
          guberniaName={gubernia.name}
          settlements={provinceSettlements}
          selectedDistrictId={districtId}
          selectedSettlementId={settlementId}
          onDistrictSelect={selectDistrict}
          onSettlementOpen={openSettlement}
          onDistrictsLoad={updateDistricts}
        />
      </div>

      {gubernia.description || gubernia.images.length > 0 ? (
        <section className={styles.reference} aria-labelledby="province-reference-heading">
          <h2 className={styles.referenceHeading} id="province-reference-heading">
            Справочные сведения
          </h2>
          {gubernia.images.length > 0 ? (
            <PostImageGallery images={gubernia.images} ariaLabel="Изображения губернии" />
          ) : null}
          {gubernia.description ? (
            <RichDocumentPreview
              body={gubernia.description}
              expanded={referenceExpanded}
              onExpandedChange={setReferenceExpanded}
            />
          ) : null}
        </section>
      ) : null}

      {isAdmin ? (
        <div className={styles.composer}>
          <AdminCreationPanel
            guberniaId={gubernia.id}
            provinces={provinces}
            settlements={settlements}
            categories={categories}
            settlementTypes={settlementTypes}
          />
        </div>
      ) : null}

      <section
        className={styles.filters}
        id="province-post-filters"
        aria-label="Фильтры сообщений"
      >
        <ProvinceFilterControls idPrefix="province-post" {...filterControlsProps} />
      </section>

      {gubernia.posts.length > 0 ? (
        <PostFeed
          posts={visiblePosts}
          guberniaId={gubernia.id}
          provinces={provinces}
          settlements={settlements}
          categories={categories}
          districtNames={districtNames}
          isAdmin={isAdmin}
          ariaLabel="Сообщения губернии"
          emptyMessage="По выбранным фильтрам сообщений нет."
          keepSidebarWhenEmpty
        />
      ) : null}
      <span className={styles.visuallyHidden} role="status">
        {hasFilters ? `Показано сообщений: ${visiblePosts.length}.` : ""}
      </span>
    </>
  );
}
