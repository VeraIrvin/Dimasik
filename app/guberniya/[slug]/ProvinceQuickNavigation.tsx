"use client";

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import Link from "next/link";
import styles from "./page.module.css";

type Props = {
  children?: ReactNode;
  /**
   * Id of the in-flow filter form: its top crossing reveals the toolbar, its
   * bottom crossing adds the compact filters.
   */
  filtersId?: string;
  /** Id of the top content anchor: trigger and target of the ↑ button. */
  upTargetId?: string;
  /**
   * Edge of the up target whose crossing of the viewport top reveals the ↑
   * button: bottom for the province map, top for a long reference section.
   */
  upVisibilityEdge?: "top" | "bottom";
  /** Contextual destination, identical to the in-flow header back link. */
  backHref?: string;
  /** Accessible name of the fixed toolbar. */
  navigationLabel?: string;
  /** Accessible name and title of the ↑ button. */
  upLabel?: string;
};

function hasCrossedViewportTop(
  rect: { readonly top: number; readonly bottom: number },
  edge: "top" | "bottom",
) {
  return rect[edge] <= 0;
}

/**
 * Fixed compact toolbar shared by the province and settlement pages: a back
 * link, an optional compact filter form, and an ↑ button returning to the top
 * content anchor. The defaults describe the province page, keeping that call
 * site unchanged.
 */
export default function ProvinceQuickNavigation({
  children,
  filtersId = "province-post-filters",
  upTargetId = "province-map",
  upVisibilityEdge = "bottom",
  backHref = "/",
  navigationLabel = "Навигация по странице губернии",
  upLabel = "К карте губернии",
}: Props) {
  const [filtersTopReached, setFiltersTopReached] = useState(false);
  const [filtersAbove, setFiltersAbove] = useState(false);
  const [upTargetAbove, setUpTargetAbove] = useState(false);
  const navigationRef = useRef<HTMLElement>(null);

  useEffect(() => {
    const filters = document.getElementById(filtersId);
    const upTarget = document.getElementById(upTargetId);
    if (!filters || !upTarget) return;

    const filtersObserver = new IntersectionObserver(([entry]) => {
      const filtersRect = entry.boundingClientRect;
      setFiltersTopReached(hasCrossedViewportTop(filtersRect, "top"));
      setFiltersAbove(hasCrossedViewportTop(filtersRect, "bottom"));
    });
    const upTargetObserver = new IntersectionObserver(([entry]) => {
      setUpTargetAbove(hasCrossedViewportTop(entry.boundingClientRect, upVisibilityEdge));
    });
    filtersObserver.observe(filters);
    upTargetObserver.observe(upTarget);

    // An instant scroll (anchor navigation, programmatic scrollTo) can move an
    // element across the viewport top without ever changing its intersection
    // state. Re-check every scroll and resize frame with the same edge rules as
    // the observers; top-edge mode can update while a tall target is visible.
    let frame = 0;
    const updateAboveStates = () => {
      frame = 0;
      const filtersRect = filters.getBoundingClientRect();
      setFiltersTopReached(hasCrossedViewportTop(filtersRect, "top"));
      setFiltersAbove(hasCrossedViewportTop(filtersRect, "bottom"));
      setUpTargetAbove(
        hasCrossedViewportTop(upTarget.getBoundingClientRect(), upVisibilityEdge),
      );
    };
    const scheduleUpdate = () => {
      if (!frame) frame = requestAnimationFrame(updateAboveStates);
    };
    updateAboveStates();
    window.addEventListener("scroll", scheduleUpdate, { passive: true });
    window.addEventListener("resize", scheduleUpdate);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("scroll", scheduleUpdate);
      window.removeEventListener("resize", scheduleUpdate);
      filtersObserver.disconnect();
      upTargetObserver.disconnect();
    };
  }, [filtersId, upTargetId, upVisibilityEdge]);

  function scrollToUpTarget() {
    document.getElementById(upTargetId)?.scrollIntoView({
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
      block: "start",
    });
  }

  const showNavigation = filtersTopReached;
  const showFilters = filtersAbove;

  useLayoutEffect(() => {
    const navigation = navigationRef.current;
    const main = navigation?.closest("main");
    if (!showNavigation || !navigation || !main) return;

    // The back-only and full-filter layouts can wrap to different heights.
    // Keep the obstruction local to this page, including after responsive resize.
    const updateOffset = () => {
      const height = navigation.getBoundingClientRect().height;
      main.style.setProperty("--page-navigation-offset", `${height > 0 ? height + 8 : 0}px`);
    };
    updateOffset();
    const observer = new ResizeObserver(updateOffset);
    observer.observe(navigation, { box: "border-box" });
    return () => {
      observer.disconnect();
      main.style.removeProperty("--page-navigation-offset");
    };
  }, [showNavigation, showFilters]);

  return (
    <>
      {showNavigation ? (
        <nav
          ref={navigationRef}
          className={`${styles.floatingNavigation} ${showFilters ? "" : styles.floatingNavigationBackOnly}`}
          aria-label={navigationLabel}
        >
          <Link className={styles.floatingBack} href={backHref}>
            ← Вернуться к карте
          </Link>
          {showFilters ? children : null}
        </nav>
      ) : null}
      {upTargetAbove ? (
        <button
          className={styles.floatingMap}
          type="button"
          onClick={scrollToUpTarget}
          aria-label={upLabel}
          title={upLabel}
        >
          ↑
        </button>
      ) : null}
    </>
  );
}
