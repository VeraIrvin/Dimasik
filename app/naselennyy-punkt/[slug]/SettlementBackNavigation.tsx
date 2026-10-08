"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import styles from "./page.module.css";

type Props = {
  /** Server-derived destination, identical to the in-flow back link. */
  href: string;
};

function isInViewport(element: HTMLElement) {
  const rect = element.getBoundingClientRect();
  return rect.bottom > 0 && rect.top < window.innerHeight;
}

/**
 * Compact back link shown once the page header scrolls away, so the reference
 * section stays returnable. Hidden while the feed rail shows its own back link,
 * or is about to when the rail controls reach the viewport, which keeps a single
 * back control visible at any scroll position.
 */
export default function SettlementBackNavigation({ href }: Props) {
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const update = () => {
      const header = document.getElementById("settlement-page-header");
      const headerAbove = header ? header.getBoundingClientRect().bottom <= 0 : false;

      // The rail renders its back link as soon as the header is away, so an
      // on-screen rail means the floating copy must stay out of the way.
      const rail = document.getElementById("settlement-rail-controls");
      const railVisible = rail ? isInViewport(rail) : false;

      setVisible(headerAbove && !railVisible);
    };

    update();
    window.addEventListener("scroll", update, { passive: true });
    window.addEventListener("resize", update);
    window.addEventListener("settlement-rail-controls-change", update);
    return () => {
      window.removeEventListener("scroll", update);
      window.removeEventListener("resize", update);
      window.removeEventListener("settlement-rail-controls-change", update);
    };
  }, []);

  if (!visible) return null;

  return (
    <nav className={styles.floatingNavigation} aria-label="Навигация по странице населённого пункта">
      <Link className={styles.floatingBack} href={href}>
        ← Вернуться к карте
      </Link>
    </nav>
  );
}
