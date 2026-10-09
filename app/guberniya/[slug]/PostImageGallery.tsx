"use client";

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { PostImage } from "@/lib/gubernia-publications";
import styles from "./PostImageGallery.module.css";

/** Focusable elements the lightbox keeps inside its dialog. */
const FOCUSABLE_SELECTOR =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

type Props = {
  images: PostImage[];
};

/** Post attachments: a thumbnail strip whose images open full size in a lightbox. */
export default function PostImageGallery({ images }: Props) {
  const count = images.length;
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  const close = useCallback(() => {
    setOpenIndex(null);
  }, []);

  const open = useCallback((index: number, trigger: HTMLElement) => {
    // Restore focus to the exact thumbnail that opened the lightbox: pointer
    // activation does not focus buttons in every browser (notably Safari), so
    // document.activeElement may be an unrelated control or the body.
    restoreFocusRef.current = trigger;
    setOpenIndex(index);
  }, []);

  const step = useCallback(
    (delta: number) => {
      setOpenIndex((current) => {
        if (current === null || count < 2) return current;
        return (current + delta + count) % count;
      });
    },
    [count],
  );

  const isOpen = openIndex !== null;

  useLayoutEffect(() => {
    if (!isOpen) {
      // The dialog is gone from the DOM. Focus the exact clicked thumbnail
      // synchronously in the layout phase: requestAnimationFrame can be paused
      // by the browser and leave focus stranded on <body>. React runs the open
      // branch's cleanup before this closed branch, so the page scroll has
      // already been restored when focus() scrolls the thumbnail into view.
      const target = restoreFocusRef.current;
      restoreFocusRef.current = null;
      target?.focus();
      return;
    }
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    closeButtonRef.current?.focus();
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
        return;
      }
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        step(-1);
        return;
      }
      if (event.key === "ArrowRight") {
        event.preventDefault();
        step(1);
        return;
      }
      if (event.key !== "Tab") return;

      const dialog = dialogRef.current;
      if (!dialog) return;
      const focusable = Array.from(dialog.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
      if (focusable.length === 0) {
        event.preventDefault();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (!(active instanceof HTMLElement) || !dialog.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
        return;
      }
      if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen, close, step]);

  if (count === 0) return null;

  return (
    <>
      <ul className={styles.gallery} aria-label="Изображения сообщения">
        {images.map((image, index) => (
          <li key={image.id}>
            <button
              className={styles.thumbnail}
              type="button"
              aria-haspopup="dialog"
              aria-label={`Открыть изображение ${index + 1} из ${count}`}
              onClick={(event) => open(index, event.currentTarget)}
            >
              <img
                className={styles.thumbnailImage}
                src={image.thumbnailUrl}
                width={image.width}
                height={image.height}
                alt=""
                loading="lazy"
              />
            </button>
          </li>
        ))}
      </ul>

      {openIndex !== null ? (
        <div
          className={styles.backdrop}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) close();
          }}
        >
          <div
            ref={dialogRef}
            className={styles.dialog}
            role="dialog"
            aria-modal="true"
            aria-label="Просмотр изображений"
          >
            <div className={styles.dialogHeader}>
              <p className={styles.counter} aria-live="polite">
                {openIndex + 1} / {count}
              </p>
              <button ref={closeButtonRef} className={styles.closeButton} type="button" onClick={close}>
                Закрыть
              </button>
            </div>
            <div className={styles.viewport}>
              <img
                className={styles.fullImage}
                src={images[openIndex].originalUrl}
                width={images[openIndex].width}
                height={images[openIndex].height}
                alt={`Изображение ${openIndex + 1} из ${count}`}
              />
            </div>
            {count > 1 ? (
              <div className={styles.navigation}>
                <button className={styles.navButton} type="button" onClick={() => step(-1)}>
                  ← Назад
                </button>
                <button className={styles.navButton} type="button" onClick={() => step(1)}>
                  Вперёд →
                </button>
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </>
  );
}
