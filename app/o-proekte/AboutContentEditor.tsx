"use client";

import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { useRouter } from "next/navigation";
import PostBody from "@/app/guberniya/[slug]/PostBody";
import PostImageGallery from "@/app/guberniya/[slug]/PostImageGallery";
import PostImageUploader, {
  type PostImageUploaderHandle,
} from "@/app/guberniya/[slug]/PostImageUploader";
import RichDocumentPreview from "@/components/RichDocumentPreview";
import RichTextField, {
  usePostDocumentEditor,
  type RichTextFieldHandle,
} from "@/components/RichTextField";
import type { PostImage } from "@/lib/gubernia-publications";
import type { PostDocument } from "@/lib/post-content";
import styles from "./AboutContentEditor.module.css";

const KEYBOARD_HINT = "Enter — новый абзац, Shift+Enter — перенос строки внутри абзаца.";

type AboutContentEditorProps = {
  initialBody: PostDocument;
  isAdmin: boolean;
};

type RichPageContentEditorProps = {
  initialBody: PostDocument | null;
  isAdmin: boolean;
  endpoint: string;
  ariaLabel: string;
  editorLabel: string;
  emptyState: ReactNode;
  emptyValidationMessage: string;
  saveErrorMessage: string;
  editLabel?: string;
  secondaryAction?: ReactNode;
  /**
   * Opts the block into the shared post-image workflow: an image picker in the
   * editor, an ordered `imageIds` array on save, and a public gallery under the
   * text. Pages that only edit rich text leave this off and keep the exact
   * text-only contract, validation included.
   */
  imageSupport?: boolean;
  /** Saved images in display order; only meaningful together with `imageSupport`. */
  initialImages?: PostImage[];
  /** Accessible name of the public gallery under the text. */
  imagesAriaLabel?: string;
  /**
   * Opens a long document as a truncated rich-text prefix (~560 characters)
   * with a «Читать далее» toggle instead of the full body. The prefix renders
   * through the same `PostBody`, keeping headings, lists, marks and links, and
   * the omitted remainder is never mounted. Documents whose text is no longer
   * than the excerpt render unchanged, and pages that always show the whole
   * body leave this off.
   */
  collapsedPreview?: boolean;
  /**
   * Element id that receives the admin row instead of the block itself, so a
   * record page can keep the row under its title while the editor stays in
   * place. Until the page's markup is ready the row is withheld, and a missing
   * slot falls back to the row's original place below the text.
   */
  adminActionsTargetId?: string;
};

/**
 * Shared read/edit workflow for public rich-text page sections. The endpoint
 * remains responsible for authorization and document normalization.
 */
export function RichPageContentEditor({
  initialBody,
  isAdmin,
  endpoint,
  ariaLabel,
  editorLabel,
  emptyState,
  emptyValidationMessage,
  saveErrorMessage,
  editLabel = "Редактировать",
  secondaryAction,
  adminActionsTargetId,
  collapsedPreview = false,
  imageSupport = false,
  initialImages = [],
  imagesAriaLabel = "Изображения",
}: RichPageContentEditorProps) {
  const router = useRouter();
  const [body, setBody] = useState<PostDocument | null>(initialBody);
  const [images, setImages] = useState<PostImage[]>(initialImages);
  const [editing, setEditing] = useState(false);
  const [status, setStatus] = useState("");
  const [previewExpanded, setPreviewExpanded] = useState(false);
  const [returnFocus, setReturnFocus] = useState(false);
  const [adminRowTarget, setAdminRowTarget] = useState<HTMLElement | null | undefined>(undefined);
  const editButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!adminActionsTargetId) return;
    // The page renders the slot; portaling keeps one state for row and editor.
    setAdminRowTarget(document.getElementById(adminActionsTargetId));
  }, [adminActionsTargetId]);

  useEffect(() => {
    if (!returnFocus || editing || !isAdmin) return;
    // Both finishing paths hide the form: send the caret back to its opener.
    editButtonRef.current?.focus();
    setReturnFocus(false);
  }, [editing, isAdmin, returnFocus]);

  function startEditing() {
    setStatus("");
    setEditing(true);
  }

  function finishEditing() {
    setReturnFocus(true);
    setEditing(false);
  }

  function handleSaved(nextBody: PostDocument | null, nextImages: PostImage[]) {
    setBody(nextBody);
    setImages(nextImages);
    setPreviewExpanded(false);
    setStatus("Изменения сохранены.");
    finishEditing();
    router.refresh();
  }

  const adminActions = isAdmin && !editing ? (
    <div className={styles.adminActions}>
      <button
        ref={editButtonRef}
        className={styles.editButton}
        type="button"
        onClick={startEditing}
      >
        {editLabel}
      </button>
      {secondaryAction}
    </div>
  ) : null;

  /** Record pages host the row in the page's own slot; everything else keeps it here. */
  function renderAdminActions() {
    if (!adminActionsTargetId || adminRowTarget === null) return adminActions;
    return adminRowTarget ? createPortal(adminActions, adminRowTarget) : null;
  }

  /** Read mode: long references open as a rich prefix, everything else as full text. */
  function renderReadContent() {
    const gallery =
      imageSupport && images.length > 0 ? (
        <PostImageGallery images={images} ariaLabel={imagesAriaLabel} />
      ) : null;
    if (!body) return gallery ?? emptyState;
    if (!collapsedPreview) {
      return (
        <>
          {gallery}
          <PostBody body={body} />
        </>
      );
    }
    return (
      <>
        {gallery}
        <RichDocumentPreview
          body={body}
          expanded={previewExpanded}
          onExpandedChange={setPreviewExpanded}
        />
      </>
    );
  }

  return (
    <section className={styles.wrapper} aria-label={ariaLabel}>
      {editing ? (
        <RichPageContentForm
          initialBody={body}
          initialImages={images}
          imageSupport={imageSupport}
          endpoint={endpoint}
          editorLabel={editorLabel}
          emptyValidationMessage={emptyValidationMessage}
          saveErrorMessage={saveErrorMessage}
          onSaved={handleSaved}
          onCancel={() => {
            setStatus("");
            finishEditing();
          }}
        />
      ) : (
        renderReadContent()
      )}

      {!editing && status ? (
        <p className={styles.status} role="status">
          {status}
        </p>
      ) : null}

      {renderAdminActions()}
    </section>
  );
}

/** Keeps the About page's established copy and API contract unchanged. */
export default function AboutContentEditor({ initialBody, isAdmin }: AboutContentEditorProps) {
  return (
    <RichPageContentEditor
      initialBody={initialBody}
      isAdmin={isAdmin}
      endpoint="/api/about-content"
      ariaLabel="Содержание страницы"
      editorLabel="Текст страницы"
      emptyState={null}
      emptyValidationMessage="Добавьте текст страницы."
      saveErrorMessage="Не удалось сохранить текст страницы. Попробуйте ещё раз."
    />
  );
}

type RichPageContentFormProps = {
  initialBody: PostDocument | null;
  endpoint: string;
  editorLabel: string;
  emptyValidationMessage: string;
  saveErrorMessage: string;
  onSaved: (body: PostDocument | null, images: PostImage[]) => void;
  onCancel: () => void;
  /** When set, the form also saves the ordered image set and mounts the picker. */
  imageSupport?: boolean;
  initialImages?: PostImage[];
};

function RichPageContentForm({
  initialBody,
  endpoint,
  editorLabel,
  emptyValidationMessage,
  saveErrorMessage,
  onSaved,
  onCancel,
  imageSupport = false,
  initialImages = [],
}: RichPageContentFormProps) {
  const editor = usePostDocumentEditor(initialBody ?? "", editorLabel);
  const richTextFieldRef = useRef<RichTextFieldHandle>(null);
  const imageUploaderRef = useRef<PostImageUploaderHandle>(null);
  const [saving, setSaving] = useState(false);
  /** True while the images are uploading or one of them failed. */
  const [imagesBlocked, setImagesBlocked] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    editor?.commands.focus();
  }, [editor]);

  async function save() {
    if (!editor || saving) return;
    const imageUploader = imageSupport ? imageUploaderRef.current : null;
    if (imageUploader?.isBlocked()) {
      setError("Дождитесь завершения загрузки изображений или удалите файлы с ошибкой.");
      return;
    }
    // Same rule as posts: empty text is valid only with images in the final set.
    const imageIds = imageUploader?.getImageIds() ?? [];
    if (!editor.getText().trim() && imageIds.length === 0) {
      setError(emptyValidationMessage);
      editor.chain().focus().run();
      return;
    }

    setSaving(true);
    setError("");
    // Until the save response lands, new uploads may already be claimed by the
    // request; while this is set, cleanup leaves them to the server TTL sweep.
    imageUploader?.setSaveInFlight(true);
    try {
      const response = await fetch(endpoint, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({
          body: editor.getJSON(),
          // The ordered final set is sent every time: kept images in order plus
          // new uploads. Omission would leave stored images untouched instead.
          ...(imageSupport ? { imageIds } : {}),
        }),
      });
      if (!response.ok) {
        setError(await errorMessage(response, saveErrorMessage));
        return;
      }

      const data = (await response.json()) as { body?: PostDocument | null; images?: PostImage[] };
      const nextImages = Array.isArray(data.images) ? data.images : [];
      if (!data.body && (!imageSupport || nextImages.length === 0)) {
        setError("Сервер вернул некорректный ответ. Попробуйте ещё раз.");
        return;
      }
      // The stored record now owns the new uploads: never clean them up as pending.
      imageUploaderRef.current?.claimAll();
      onSaved(data.body ?? null, nextImages);
    } catch {
      setError("Не удалось связаться с сервером.");
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

  function handleCancel() {
    if (saving) return;
    // Cancel discards only the new pending uploads; saved images are untouched
    // and staged removals are dropped without being applied.
    imageUploaderRef.current?.cleanupPending();
    onCancel();
  }

  function handleKeyDown(event: KeyboardEvent<HTMLFormElement>) {
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
    handleCancel();
  }

  return (
    <form className={styles.form} onSubmit={handleSubmit} onKeyDown={handleKeyDown} noValidate>
      <span className={styles.bodyLabel}>{editorLabel}</span>
      <RichTextField
        ref={richTextFieldRef}
        editor={editor}
        hint={
          imageSupport
            ? "Текст необязателен, если приложено изображение. Enter — новый абзац, Shift+Enter — перенос строки внутри абзаца."
            : KEYBOARD_HINT
        }
      />

      {imageSupport ? (
        <PostImageUploader
          ref={imageUploaderRef}
          initialImages={initialImages}
          disabled={saving}
          onBlockedChange={setImagesBlocked}
        />
      ) : null}

      {error ? (
        <p className={styles.error} role="alert">
          {error}
        </p>
      ) : null}

      <div className={styles.actions}>
        <button
          className={styles.primaryButton}
          type="submit"
          disabled={saving || !editor || (imageSupport && imagesBlocked)}
        >
          {saving ? "Сохраняем…" : "Сохранить"}
        </button>
        <button className={styles.cancelButton} type="button" onClick={handleCancel} disabled={saving}>
          Отмена
        </button>
      </div>
    </form>
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
