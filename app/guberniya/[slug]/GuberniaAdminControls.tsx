"use client";

import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useRouter } from "next/navigation";
import RichTextField, {
  usePostDocumentEditor,
  type RichTextFieldHandle,
} from "@/components/RichTextField";
import type { PostImage } from "@/lib/gubernia-publications";
import type { PostDocument } from "@/lib/post-content";
import PostImageUploader, { type PostImageUploaderHandle } from "./PostImageUploader";
import styles from "./GuberniaAdminControls.module.css";

type Props = {
  id: string;
  slug: string;
  description: PostDocument | null;
  /** Saved reference images in display order; empty when there are none. */
  images: PostImage[];
};

type UpdateResponse = {
  slug?: unknown;
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

export default function GuberniaAdminControls({ id, slug, description, images }: Props) {
  const router = useRouter();
  const [editing, setEditing] = useState(false);
  const [saved, setSaved] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const editTriggerRef = useRef<HTMLButtonElement>(null);
  const deleteTriggerRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  function openEdit() {
    setSaved(false);
    setEditing(true);
  }

  function cancelEdit() {
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

  /** A stored slug change moves the published page to its new address. */
  function handleSaved(nextSlug: string) {
    if (nextSlug !== slug) {
      router.push(`/guberniya/${encodeURIComponent(nextSlug)}`);
      return;
    }
    setEditing(false);
    setSaved(true);
    router.refresh();
    requestAnimationFrame(() => editTriggerRef.current?.focus());
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
    <section className={styles.panel} aria-label="Управление губернией">
      {editing ? (
        <GuberniaEditForm
          id={id}
          initialSlug={slug}
          initialDescription={description}
          initialImages={images}
          onSaved={handleSaved}
          onCancel={cancelEdit}
        />
      ) : (
        <>
          {saved ? (
            <p className={styles.status} role="status">
              Сохранено.
            </p>
          ) : null}
          <div className={styles.adminActions}>
            <button ref={editTriggerRef} className={styles.editButton} type="button" onClick={openEdit}>
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

type GuberniaEditFormProps = {
  id: string;
  initialSlug: string;
  initialDescription: PostDocument | null;
  initialImages: PostImage[];
  onSaved: (slug: string) => void;
  onCancel: () => void;
};

/**
 * Mounted only while the admin edits the province: mounting seeds the slug
 * field, the rich-text editor and the image picker, so cancelling discards
 * them all and the next open starts from the stored values. A blank editor
 * stores no description.
 */
function GuberniaEditForm({
  id,
  initialSlug,
  initialDescription,
  initialImages,
  onSaved,
  onCancel,
}: GuberniaEditFormProps) {
  const editor = usePostDocumentEditor(initialDescription ?? "", "Описание");
  const richTextFieldRef = useRef<RichTextFieldHandle>(null);
  const imageUploaderRef = useRef<PostImageUploaderHandle>(null);
  const slugRef = useRef<HTMLInputElement>(null);
  const [slugValue, setSlugValue] = useState(initialSlug);
  const [saving, setSaving] = useState(false);
  /** True while the images are uploading or one of them failed. */
  const [imagesBlocked, setImagesBlocked] = useState(false);
  const [saveError, setSaveError] = useState("");

  // The address stays the first field of the form, so opening it focuses there.
  useEffect(() => {
    slugRef.current?.focus();
  }, []);

  async function save() {
    if (!editor || saving) return;
    const imageUploader = imageUploaderRef.current;
    if (imageUploader?.isBlocked()) {
      setSaveError("Дождитесь завершения загрузки изображений или удалите файлы с ошибкой.");
      return;
    }
    const nextSlug = slugValue.trim();
    if (!nextSlug) {
      setSaveError("Адрес страницы не может быть пустым.");
      return;
    }
    // A blank editor means «no description»; the server also folds an
    // invisible document to null, so the field carries a document or null.
    const nextDescription = editor.getText().trim() ? editor.getJSON() : null;
    // The ordered final set is sent every time: kept images in order plus new
    // uploads. Omitting it would leave stored images untouched instead.
    const imageIds = imageUploader?.getImageIds() ?? [];

    setSaving(true);
    setSaveError("");
    // Until the save response lands, new uploads may already be claimed by the
    // request; while this is set, cleanup leaves them to the server TTL sweep.
    imageUploader?.setSaveInFlight(true);
    try {
      const response = await fetch(`/api/gubernias/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ slug: nextSlug, description: nextDescription, imageIds }),
      });
      if (!response.ok) {
        setSaveError(await errorMessage(response, "Не удалось сохранить изменения. Попробуйте ещё раз."));
        return;
      }

      let data: UpdateResponse = {};
      try {
        data = (await response.json()) as UpdateResponse;
      } catch {
        /* Keep the locally entered address if the body is unreadable. */
      }
      // The stored province now owns the new uploads: never clean them up as pending.
      imageUploaderRef.current?.claimAll();
      onSaved(typeof data.slug === "string" && data.slug ? data.slug : nextSlug);
    } catch {
      setSaveError("Не удалось связаться с сервером.");
    } finally {
      // A settled request no longer races cleanup: failed saves must fall back
      // to normal pending deletion on cancel or unmount.
      imageUploader?.setSaveInFlight(false);
      setSaving(false);
    }
  }

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void save();
  }

  function cancel() {
    if (saving) return;
    // Cancel discards only the new pending uploads; saved images are untouched
    // and staged removals are dropped without being applied.
    imageUploaderRef.current?.cleanupPending();
    onCancel();
  }

  function handleKeyDown(event: ReactKeyboardEvent<HTMLFormElement>) {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void save();
      return;
    }
    if (event.key !== "Escape" || saving) return;
    if (richTextFieldRef.current?.isLinkFieldOpen()) {
      event.preventDefault();
      richTextFieldRef.current.closeLinkField();
      return;
    }
    const target = event.target;
    // Inside the body Escape belongs to the editor, not to the whole form.
    if (editor && target instanceof Node && editor.view.dom.contains(target)) return;
    event.preventDefault();
    cancel();
  }

  return (
    <form className={styles.form} onSubmit={handleSubmit} onKeyDown={handleKeyDown}>
      <label htmlFor="gubernia-slug">Адрес страницы</label>
      <input
        ref={slugRef}
        id="gubernia-slug"
        name="slug"
        value={slugValue}
        onChange={(event) => setSlugValue(event.target.value)}
        placeholder="tulskaya"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        required
      />
      <span className={styles.descriptionLabel}>Описание</span>
      <RichTextField
        ref={richTextFieldRef}
        editor={editor}
        hint="Описание необязательно. Enter — новый абзац, Shift+Enter — перенос строки внутри абзаца."
      />

      <PostImageUploader
        ref={imageUploaderRef}
        initialImages={initialImages}
        disabled={saving}
        onBlockedChange={setImagesBlocked}
      />

      {saveError ? (
        <p className={styles.error} role="alert">
          {saveError}
        </p>
      ) : null}
      <div className={styles.actions}>
        <button
          className={styles.primaryButton}
          type="submit"
          disabled={saving || !editor || imagesBlocked}
        >
          {saving ? "Сохраняем…" : "Сохранить"}
        </button>
        <button className={styles.cancelButton} type="button" onClick={cancel} disabled={saving}>
          Отмена
        </button>
      </div>
    </form>
  );
}
