"use client";

import { useEffect, useRef, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import styles from "./GuberniaAdminControls.module.css";

type Props = {
  id: string;
  slug: string;
  description: string;
};

type UpdateResponse = {
  id?: unknown;
  slug?: unknown;
  description?: unknown;
};

async function errorMessage(response: Response, fallback: string) {
  try {
    const data = (await response.json()) as { error?: unknown };
    if (typeof data.error === "string" && data.error.trim()) return data.error;
  } catch {
    /* Non-JSON error body: fall back to the generic message. */
  }
  return fallback;
}

export default function GuberniaAdminControls({ id, slug, description }: Props) {
  const router = useRouter();
  const [editing, setEditing] = useState(false);
  const [slugValue, setSlugValue] = useState(slug);
  const [descriptionValue, setDescriptionValue] = useState(description);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState("");
  const [saved, setSaved] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const editTriggerRef = useRef<HTMLButtonElement>(null);
  const slugRef = useRef<HTMLInputElement>(null);
  const deleteTriggerRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  // Opening the form moves focus into the first field; closing it returns focus
  // to the trigger, so the keyboard path stays on the controls either way.
  useEffect(() => {
    if (editing) slugRef.current?.focus();
  }, [editing]);

  function openEdit() {
    setSlugValue(slug);
    setDescriptionValue(description);
    setSaveError("");
    setSaved(false);
    setEditing(true);
  }

  function cancelEdit() {
    if (saving) return;
    setSlugValue(slug);
    setDescriptionValue(description);
    setSaveError("");
    setSaved(false);
    setEditing(false);
    requestAnimationFrame(() => editTriggerRef.current?.focus());
  }

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

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (saving || deleting) return;
    const nextSlug = slugValue.trim();
    if (!nextSlug) {
      setSaved(false);
      setSaveError("Адрес страницы не может быть пустым.");
      return;
    }

    setSaving(true);
    setSaved(false);
    setSaveError("");
    try {
      const response = await fetch(`/api/gubernias/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ slug: nextSlug, description: descriptionValue }),
      });
      if (!response.ok) {
        setSaveError(await errorMessage(response, "Не удалось сохранить изменения. Попробуйте ещё раз."));
        return;
      }

      let data: UpdateResponse = {};
      try {
        data = (await response.json()) as UpdateResponse;
      } catch {
        /* Keep the locally entered values if the body is unreadable. */
      }
      const savedSlug = typeof data.slug === "string" && data.slug ? data.slug : nextSlug;
      if (typeof data.description === "string") setDescriptionValue(data.description);
      setSlugValue(savedSlug);

      if (savedSlug !== slug) {
        router.push(`/guberniya/${encodeURIComponent(savedSlug)}`);
      } else {
        setEditing(false);
        setSaved(true);
        router.refresh();
        requestAnimationFrame(() => editTriggerRef.current?.focus());
      }
    } catch {
      setSaveError("Не удалось связаться с сервером.");
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete() {
    if (deleting) return;
    setDeleting(true);
    setDeleteError("");
    try {
      const response = await fetch(`/api/gubernias/${encodeURIComponent(id)}`, {
        method: "DELETE",
        credentials: "same-origin",
      });
      if (!response.ok) {
        setDeleteError(await errorMessage(response, "Не удалось удалить страницу. Попробуйте ещё раз."));
        return;
      }
      setConfirmOpen(false);
      router.push("/");
    } catch {
      setDeleteError("Не удалось связаться с сервером.");
    } finally {
      setDeleting(false);
    }
  }

  return (
    <section className={styles.panel} aria-labelledby="gubernia-admin-title">
      <h2 id="gubernia-admin-title" className={styles.heading}>
        Управление губернией
      </h2>

      {editing ? (
        <form
          className={styles.form}
          onSubmit={handleSave}
          onKeyDown={(event) => {
            if (event.key === "Escape" && !saving) {
              event.preventDefault();
              cancelEdit();
            }
          }}
        >
          <label htmlFor="gubernia-slug">Адрес страницы</label>
          <input
            ref={slugRef}
            id="gubernia-slug"
            name="slug"
            value={slugValue}
            onChange={(event) => {
              setSlugValue(event.target.value);
              setSaved(false);
            }}
            placeholder="tulskaya"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            required
          />
          <label htmlFor="gubernia-description">Описание</label>
          <textarea
            id="gubernia-description"
            name="description"
            rows={6}
            value={descriptionValue}
            onChange={(event) => {
              setDescriptionValue(event.target.value);
              setSaved(false);
            }}
          />
          {saveError ? (
            <p className={styles.error} role="alert">
              {saveError}
            </p>
          ) : null}
          <div className={styles.actions}>
            <button className={styles.primaryButton} type="submit" disabled={saving}>
              {saving ? "Сохраняем…" : "Сохранить"}
            </button>
            <button className={styles.cancelButton} type="button" onClick={cancelEdit} disabled={saving}>
              Отмена
            </button>
          </div>
        </form>
      ) : (
        <>
          {saved ? (
            <p className={styles.status} role="status">
              Сохранено.
            </p>
          ) : null}
          <div className={styles.actions}>
            <button ref={editTriggerRef} className={styles.primaryButton} type="button" onClick={openEdit}>
              Редактировать губернию
            </button>
            <button
              ref={deleteTriggerRef}
              className={styles.deleteButton}
              type="button"
              onClick={() => {
                setDeleteError("");
                setConfirmOpen(true);
              }}
            >
              Удалить губернию
            </button>
          </div>
        </>
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
            aria-labelledby="gubernia-delete-title"
            aria-describedby="gubernia-delete-question"
          >
            <h3 id="gubernia-delete-title">Удаление страницы</h3>
            <p id="gubernia-delete-question" className={styles.question}>
              Вы точно хотите удалить страницу губернии?
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
    </section>
  );
}
