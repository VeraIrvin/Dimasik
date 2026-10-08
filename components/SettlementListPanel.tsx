"use client";

import Link from "next/link";
import { useEffect, useId, useMemo, useState } from "react";
import type { PublishedOption, Settlement } from "@/lib/gubernia-publications";
import styles from "./SettlementListPanel.module.css";

type SettlementListPanelProps = {
  /** Every settlement of the published governorates. */
  settlements: Settlement[];
  provinces: PublishedOption[];
  /** Settlement highlighted on the map; the matching row mirrors the highlight. */
  hoveredId: string | null;
  onHover: (id: string | null) => void;
  /** Admin-only creation entry. The control renders only when provided. */
  onAdd?: () => void;
};

type SettlementRow = {
  id: string;
  name: string;
  url: string;
  provinceName: string;
  /** Identical names need the province label to be told apart. */
  needsProvince: boolean;
};

export default function SettlementListPanel({
  settlements,
  provinces,
  hoveredId,
  onHover,
  onAdd,
}: SettlementListPanelProps) {
  const headingId = useId();
  const searchId = useId();
  const [query, setQuery] = useState("");

  const rows = useMemo((): SettlementRow[] => {
    const provinceNames: Record<string, string> = {};
    for (const province of provinces) provinceNames[province.id] = province.name;
    const nameCounts = new Map<string, number>();
    for (const settlement of settlements) {
      nameCounts.set(settlement.name, (nameCounts.get(settlement.name) ?? 0) + 1);
    }
    return settlements
      .map((settlement) => ({
        id: settlement.id,
        name: settlement.name,
        url: settlement.url,
        provinceName: provinceNames[settlement.guberniaId] ?? "",
        needsProvince: (nameCounts.get(settlement.name) ?? 0) > 1,
      }))
      .sort((left, right) => {
        const byName = left.name.localeCompare(right.name, "ru");
        if (byName !== 0) return byName;
        const byProvince = left.provinceName.localeCompare(right.provinceName, "ru");
        if (byProvince !== 0) return byProvince;
        return left.id.localeCompare(right.id, "ru");
      });
  }, [provinces, settlements]);

  // Case-insensitive name filter; a blank or whitespace-only query shows all.
  const normalizedQuery = query.trim().toLocaleLowerCase("ru");
  const visibleRows = useMemo(() => {
    if (normalizedQuery.length === 0) return rows;
    return rows.filter((row) => row.name.toLocaleLowerCase("ru").includes(normalizedQuery));
  }, [rows, normalizedQuery]);

  // When the query hides the mirrored row, drop that list-origin highlight so
  // the list never shows a stale one. A map-origin hover stays: the guard in
  // the map's setSettlementHover ignores this list-side clear.
  useEffect(() => {
    if (hoveredId !== null && !visibleRows.some((row) => row.id === hoveredId)) {
      onHover(null);
    }
  }, [hoveredId, visibleRows, onHover]);

  return (
    <section className={styles.panel} aria-labelledby={headingId}>
      <div className={styles.panelHeading}>
        <h2 id={headingId}>Населённые пункты</h2>
      </div>

      <div className={styles.searchField}>
        <label className={styles.visuallyHidden} htmlFor={searchId}>
          Найти населённый пункт
        </label>
        {/* `size` keeps the input's intrinsic width tiny, so the shrink-to-fit
            panel stays sized by the heading and the rows instead of the field. */}
        <input
          id={searchId}
          className={styles.searchInput}
          type="search"
          size={1}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Найти населённый пункт"
          autoComplete="off"
        />
      </div>

      {rows.length === 0 ? (
        <p className={styles.empty}>Населённые пункты ещё не добавлены.</p>
      ) : visibleRows.length === 0 ? (
        <p className={styles.empty}>Ничего не найдено по вашему запросу.</p>
      ) : (
        <ul className={styles.list}>
          {visibleRows.map((row) => (
            <li key={row.id}>
              {/* The origin flag lets the settlement page return to the list tab. */}
              <Link
                className={
                  hoveredId === row.id ? `${styles.row} ${styles.rowHovered}` : styles.row
                }
                href={`${row.url}?from=settlements`}
                prefetch={false}
                onMouseEnter={() => onHover(row.id)}
                onMouseLeave={(event) => {
                  // A focused row keeps the highlight until focus leaves it.
                  if (event.currentTarget !== document.activeElement) onHover(null);
                }}
                onFocus={() => onHover(row.id)}
                onBlur={() => onHover(null)}
              >
                <span className={styles.rowName}>{row.name}</span>
                {row.needsProvince && row.provinceName ? (
                  <span className={styles.rowProvince}>{row.provinceName}</span>
                ) : null}
              </Link>
            </li>
          ))}
        </ul>
      )}

      {onAdd ? (
        <div className={styles.addRow}>
          <button className={styles.addButton} type="button" onClick={onAdd}>
            + Добавить населённый пункт
          </button>
        </div>
      ) : null}
    </section>
  );
}
