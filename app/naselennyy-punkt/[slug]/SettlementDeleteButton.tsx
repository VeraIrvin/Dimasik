"use client";

import { useEffect, useId, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import styles from "./SettlementDeleteButton.module.css";

type SettlementDeleteButtonProps = {
  settlementId: string;
  guberniaId: string;
  settlementName: string;
  backHref: string;
};

/**
 * Admin-only removal of the settlement itself, rendered next to the reference
 * editor. The server keeps the admin gate; success lands back on the origin map
 * because the settlement page stops resolving after the delete.
 */
export default function SettlementDeleteButton({
  settlementId,
  guberniaId,
  settlementName,
  backHref,
}: SettlementDeleteButtonProps) {
  const router = useRouter();
  const dialogTitleId = useId();
  const dialogQuestionId = useId();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const deleteTriggerRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  function closeConfirm() {
    if (deleting) return;
    setConfirmOpen(false);
    setDeleteError("");
    requestAnimationFrame(() => deleteTriggerRef.current?.focus());
  }

  useEffect(() => {
    if (!confirmOpen) return;
    cancelRef.current?.focus();

    function handleDialogKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeConfirm();
        return;
      }
      if (event.key !== "Tab") return;
      const controls = dialogRef.current?.querySelectorAll<HTMLElement>("button:not(:disabled)");
      if (!controls?.length) return;
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", handleDialogKeyDown);
    return () => document.removeEventListener("keydown", handleDialogKeyDown);
  }, [confirmOpen, deleting]);

  async function handleDelete() {
    if (deleting) return;
    setDeleting(true);
    setDeleteError("");
    try {
      const response = await fetch(
        `/api/gubernias/${encodeURIComponent(guberniaId)}/settlements/${encodeURIComponent(settlementId)}`,
        { method: "DELETE", credentials: "same-origin" },
      );
      if (!response.ok) {
        setDeleteError(
          await errorMessage(response, "Не удалось удалить населённый пункт. Попробуйте ещё раз."),
        );
        return;
      }
      // The page is gone, so return to the map/list the admin came from.
      setConfirmOpen(false);
      router.push(backHref);
      router.refresh();
    } catch {
      setDeleteError("Не удалось связаться с сервером.");
    } finally {
      setDeleting(false);
    }
  }

  return (
    <>
      <button
        ref={deleteTriggerRef}
        className={styles.deleteButton}
        type="button"
        aria-haspopup="dialog"
        onClick={() => {
          setDeleteError("");
          setConfirmOpen(true);
        }}
      >
        Удалить населённый пункт
      </button>

      {confirmOpen ? (
        <div
          className={styles.backdrop}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) closeConfirm();
          }}
        >
          <div
            ref={dialogRef}
            className={styles.dialog}
            role="dialog"
            aria-modal="true"
            aria-labelledby={dialogTitleId}
            aria-describedby={dialogQuestionId}
          >
            <h3 id={dialogTitleId}>Удаление населённого пункта</h3>
            <p id={dialogQuestionId} className={styles.question}>
              Вы точно хотите удалить населённый пункт «{settlementName}»? Справочные сведения
              будут удалены. Сообщения останутся на странице губернии, но потеряют привязку к
              населённому пункту.
            </p>
            {deleteError ? (
              <p className={styles.error} role="alert">
                {deleteError}
              </p>
            ) : null}
            <div className={styles.dialogActions}>
              <button
                ref={cancelRef}
                className={styles.cancelButton}
                type="button"
                onClick={closeConfirm}
                disabled={deleting}
              >
                Отмена
              </button>
              <button
                className={styles.confirmButton}
                type="button"
                onClick={handleDelete}
                disabled={deleting}
              >
                {deleting ? "Удаляем…" : "Удалить"}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}

async function errorMessage(response: Response, fallback: string) {
  try {
    const data = (await response.json()) as { error?: unknown };
    if (typeof data.error === "string" && data.error.trim()) return data.error;
  } catch {
    /* Non-JSON error body: fall back to the generic message. */
  }
  return fallback;
}
