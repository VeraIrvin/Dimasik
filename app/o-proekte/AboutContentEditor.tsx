"use client";

import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { useRouter } from "next/navigation";
import PostBody from "@/app/guberniya/[slug]/PostBody";
import RichTextField, {
  usePostDocumentEditor,
  type RichTextFieldHandle,
} from "@/components/RichTextField";
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
}: RichPageContentEditorProps) {
  const router = useRouter();
  const [body, setBody] = useState<PostDocument | null>(initialBody);
  const [editing, setEditing] = useState(false);
  const [status, setStatus] = useState("");
  const [returnFocus, setReturnFocus] = useState(false);
  const editButtonRef = useRef<HTMLButtonElement>(null);

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

  function handleSaved(nextBody: PostDocument) {
    setBody(nextBody);
    setStatus("Изменения сохранены.");
    finishEditing();
    router.refresh();
  }

  return (
    <section className={styles.wrapper} aria-label={ariaLabel}>
      {editing ? (
        <RichPageContentForm
          initialBody={body}
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
      ) : body ? (
        <PostBody body={body} />
      ) : (
        emptyState
      )}

      {!editing && status ? (
        <p className={styles.status} role="status">
          {status}
        </p>
      ) : null}

      {isAdmin && !editing ? (
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
      ) : null}
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
  onSaved: (body: PostDocument) => void;
  onCancel: () => void;
};

function RichPageContentForm({
  initialBody,
  endpoint,
  editorLabel,
  emptyValidationMessage,
  saveErrorMessage,
  onSaved,
  onCancel,
}: RichPageContentFormProps) {
  const editor = usePostDocumentEditor(initialBody ?? "", editorLabel);
  const richTextFieldRef = useRef<RichTextFieldHandle>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    editor?.commands.focus();
  }, [editor]);

  async function save() {
    if (!editor || saving) return;
    if (!editor.getText().trim()) {
      setError(emptyValidationMessage);
      editor.chain().focus().run();
      return;
    }

    setSaving(true);
    setError("");
    try {
      const response = await fetch(endpoint, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ body: editor.getJSON() }),
      });
      if (!response.ok) {
        setError(await errorMessage(response, saveErrorMessage));
        return;
      }

      const data = (await response.json()) as { body?: PostDocument };
      if (!data.body) {
        setError("Сервер вернул некорректный ответ. Попробуйте ещё раз.");
        return;
      }
      onSaved(data.body);
    } catch {
      setError("Не удалось связаться с сервером.");
    } finally {
      setSaving(false);
    }
  }

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void save();
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
    onCancel();
  }

  return (
    <form className={styles.form} onSubmit={handleSubmit} onKeyDown={handleKeyDown} noValidate>
      <span className={styles.bodyLabel}>{editorLabel}</span>
      <RichTextField ref={richTextFieldRef} editor={editor} hint={KEYBOARD_HINT} />

      {error ? (
        <p className={styles.error} role="alert">
          {error}
        </p>
      ) : null}

      <div className={styles.actions}>
        <button className={styles.primaryButton} type="submit" disabled={saving || !editor}>
          {saving ? "Сохраняем…" : "Сохранить"}
        </button>
        <button className={styles.cancelButton} type="button" onClick={onCancel} disabled={saving}>
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
