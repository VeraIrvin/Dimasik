"use client";

import { useEffect, useState, type ReactNode } from "react";
import Link from "next/link";
import styles from "./page.module.css";

type Props = {
  children?: ReactNode;
};

export default function ProvinceQuickNavigation({ children }: Props) {
  const [headerAbove, setHeaderAbove] = useState(false);
  const [filtersInView, setFiltersInView] = useState(true);
  const [mapAbove, setMapAbove] = useState(false);

  useEffect(() => {
    const header = document.getElementById("province-page-header");
    const filters = document.getElementById("province-post-filters");
    const map = document.getElementById("province-map");
    if (!header || !filters || !map) return;

    const headerObserver = new IntersectionObserver(([entry]) => {
      setHeaderAbove(!entry.isIntersecting && entry.boundingClientRect.bottom <= 0);
    });
    const filtersObserver = new IntersectionObserver(([entry]) => {
      setFiltersInView(entry.isIntersecting);
    });
    const mapObserver = new IntersectionObserver(([entry]) => {
      setMapAbove(!entry.isIntersecting && entry.boundingClientRect.bottom <= 0);
    });
    headerObserver.observe(header);
    filtersObserver.observe(filters);
    mapObserver.observe(map);
    return () => {
      headerObserver.disconnect();
      filtersObserver.disconnect();
      mapObserver.disconnect();
    };
  }, []);

  function scrollToProvinceMap() {
    document.getElementById("province-map")?.scrollIntoView({
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
      block: "start",
    });
  }

  const showFilters = headerAbove && !filtersInView;

  return (
    <>
      {headerAbove ? (
        <nav
          className={`${styles.floatingNavigation} ${showFilters ? "" : styles.floatingNavigationBackOnly}`}
          aria-label="Навигация по странице губернии"
        >
          <Link className={styles.floatingBack} href="/">
            ← Вернуться к карте
          </Link>
          {showFilters ? children : null}
        </nav>
      ) : null}
      {mapAbove ? (
        <button
          className={styles.floatingMap}
          type="button"
          onClick={scrollToProvinceMap}
          aria-label="К карте губернии"
          title="К карте губернии"
        >
          ↑
        </button>
      ) : null}
    </>
  );
}
