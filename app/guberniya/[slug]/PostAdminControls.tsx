"use client";

import { useEffect, useId, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import type { GuberniaPost, PublishedOption, Settlement } from "@/lib/gubernia-publications";
import ProvincePostEditor from "./ProvincePostEditor";
import styles from "./PostAdminControls.module.css";

type Props = {
  guberniaId: string;
  post: GuberniaPost;
  provinces: PublishedOption[];
  settlements: Settlement[];
  categories: string[];
};

export default function PostAdminControls({
  guberniaId,
  post,
  provinces,
  settlements,
  categories,
}: Props) {
  const router = useRouter();
  const dialogTitleId = useId();
  const dialogQuestionId = useId();
  const [editing, setEditing] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const editTriggerRef = useRef<HTMLButtonElement>(null);
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
        `/api/gubernias/${encodeURIComponent(guberniaId)}/posts/${encodeURIComponent(post.id)}`,
        { method: "DELETE", credentials: "same-origin" },
      );
      if (!response.ok) {
        setDeleteError(await errorMessage(response, "Не удалось удалить сообщение. Попробуйте ещё раз."));
        return;
      }
      // The post disappears from the refreshed list, so the dialog just closes.
      setConfirmOpen(false);
      router.refresh();
    } catch {
      setDeleteError("Не удалось связаться с сервером.");
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className={styles.controls}>
      {editing ? (
        <ProvincePostEditor
          guberniaId={guberniaId}
          provinces={provinces}
          settlements={settlements}
          categories={categories}
          post={post}
          onDone={() => setEditing(false)}
        />
      ) : (
        <div className={styles.actions}>
          <button ref={editTriggerRef} className={styles.editButton} type="button" onClick={() => setEditing(true)}>
            Редактировать
          </button>
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
            Удалить
          </button>
        </div>
      )}

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
            <h3 id={dialogTitleId}>Удаление сообщения</h3>
            <p id={dialogQuestionId} className={styles.question}>
              Вы точно хотите удалить сообщение «{post.title}»?
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
              <button className={styles.confirmButton} type="button" onClick={handleDelete} disabled={deleting}>
                {deleting ? "Удаляем…" : "Удалить"}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
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
