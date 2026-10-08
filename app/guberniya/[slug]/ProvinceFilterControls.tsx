import type { Settlement } from "@/lib/gubernia-publications";
import styles from "./page.module.css";

export type DistrictOption = { id: string; name: string };

type Props = {
  idPrefix: string;
  years: string[];
  categories: string[];
  year: string;
  category: string;
  hasFilters: boolean;
  compact?: boolean;
  /** District and settlement selects; omit both for a year/category-only form. */
  districts?: DistrictOption[];
  settlements?: Settlement[];
  districtId?: string | null;
  settlementId?: string | null;
  onDistrictChange?: (districtId: string | null) => void;
  onSettlementChange?: (settlementId: string | null) => void;
  onYearChange: (year: string) => void;
  onCategoryChange: (category: string) => void;
  onReset: () => void;
};

export default function ProvinceFilterControls({
  idPrefix,
  districts,
  settlements,
  years,
  categories,
  districtId,
  settlementId,
  year,
  category,
  hasFilters,
  compact = false,
  onDistrictChange,
  onSettlementChange,
  onYearChange,
  onCategoryChange,
  onReset,
}: Props) {
  const showDistrictField = Boolean(districts && onDistrictChange);
  const showSettlementField = Boolean(settlements && onSettlementChange);
  // The settlement feed keeps only year and category, so the geo grid columns collapse.
  const withoutGeoFields = !compact && !showDistrictField && !showSettlementField;
  const containerClassName = [
    styles.filterControls,
    compact ? styles.compactFilterControls : "",
    withoutGeoFields ? styles.filterControlsNoGeo : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={containerClassName}>
      {districts && onDistrictChange ? (
        <div className={styles.filterField}>
          <label htmlFor={`${idPrefix}-district`}>Уезд</label>
          <select
            id={`${idPrefix}-district`}
            value={districtId ?? ""}
            onChange={(event) => onDistrictChange?.(event.target.value || null)}
          >
            <option value="">Все уезды</option>
            {districts.map((district) => (
              <option key={district.id} value={district.id}>{district.name}</option>
            ))}
          </select>
        </div>
      ) : null}
      {settlements && onSettlementChange ? (
        <div className={styles.filterField}>
          <label htmlFor={`${idPrefix}-settlement`}>Населённый пункт</label>
          <select
            id={`${idPrefix}-settlement`}
            value={settlementId ?? ""}
            onChange={(event) => onSettlementChange?.(event.target.value || null)}
          >
            <option value="">Все населённые пункты</option>
            {settlements.map((settlement) => (
              <option key={settlement.id} value={settlement.id}>{settlement.name}</option>
            ))}
          </select>
        </div>
      ) : null}
      <div className={styles.filterField}>
        <label htmlFor={`${idPrefix}-year`}>Год</label>
        <select id={`${idPrefix}-year`} value={year} onChange={(event) => onYearChange(event.target.value)}>
          <option value="">Все годы</option>
          {years.map((option) => <option key={option} value={option}>{option}</option>)}
        </select>
      </div>
      <div className={styles.filterField}>
        <label htmlFor={`${idPrefix}-category`}>Категория</label>
        <select
          id={`${idPrefix}-category`}
          value={category}
          onChange={(event) => onCategoryChange(event.target.value)}
        >
          <option value="">Все категории</option>
          {categories.map((option) => <option key={option} value={option}>{option}</option>)}
        </select>
      </div>
      <button className={styles.resetFilters} type="button" onClick={onReset} disabled={!hasFilters}>
        Сбросить фильтры
      </button>
    </div>
  );
}
