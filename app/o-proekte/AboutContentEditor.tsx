"use client";

import {
  useEffect,
  useId,
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
const SETTLEMENT_NAME_MAX_LENGTH = 200;

type Coordinates = {
  latitude: number;
  longitude: number;
};

/** Mirrors the settlement creation field's accepted "latitude, longitude" format. */
function parseCoordinates(raw: string): Coordinates | null {
  const parts = raw.split(",");
  if (parts.length !== 2) return null;
  const latitudePart = parts[0].trim();
  const longitudePart = parts[1].trim();
  if (!latitudePart || !longitudePart) return null;
  const latitude = Number(latitudePart);
  const longitude = Number(longitudePart);
  if (!Number.isFinite(latitude) || !Number.isFinite(longitude)) return null;
  if (latitude < -90 || latitude > 90 || longitude < -180 || longitude > 180) return null;
  return { latitude, longitude };
}
function documentHasVisibleText(document: PostDocument | null) {
  if (!document) return false;
  return nodesHaveVisibleText(document.content);
}

function nodesHaveVisibleText(nodes: PostDocument["content"]): boolean {
  for (const node of nodes) {
    if (node.type === "text" && node.text?.trim()) return true;
    if (node.content && nodesHaveVisibleText(node.content)) return true;
  }
  return false;
}



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
   * Opts settlement references into editing the record name in the same
   * request. Other shared-editor consumers keep their existing text contract.
   */
  settlementName?: string;
  /** Opts the settlement form into coordinates/type editing in the same PATCH. */
  settlementCoordinates?: string;
  settlementType?: string | null;
  settlementTypes?: string[];
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
  settlementName,
  settlementCoordinates,
  settlementType,
  settlementTypes = [],
}: RichPageContentEditorProps) {
  const router = useRouter();
  const [body, setBody] = useState<PostDocument | null>(initialBody);
  const [images, setImages] = useState<PostImage[]>(initialImages);
  const [currentSettlementName, setCurrentSettlementName] = useState(settlementName ?? "");
  const [currentSettlementCoordinates, setCurrentSettlementCoordinates] = useState(
    settlementCoordinates ?? "",
  );
  const [currentSettlementType, setCurrentSettlementType] = useState<string | null>(
    settlementType ?? null,
  );
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
    if (settlementName !== undefined) setCurrentSettlementName(settlementName);
  }, [settlementName]);

  useEffect(() => {
    if (settlementCoordinates !== undefined) {
      setCurrentSettlementCoordinates(settlementCoordinates);
      setCurrentSettlementType(settlementType ?? null);
    }
  }, [settlementCoordinates, settlementType]);

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

  function handleSaved(
    nextBody: PostDocument | null,
    nextImages: PostImage[],
    nextSettlementName?: string,
    nextSettlementCoordinates?: string,
    nextSettlementType?: string | null,
  ) {
    setBody(nextBody);
    setImages(nextImages);
    if (nextSettlementName !== undefined) setCurrentSettlementName(nextSettlementName);
    if (nextSettlementCoordinates !== undefined) {
      setCurrentSettlementCoordinates(nextSettlementCoordinates);
      setCurrentSettlementType(nextSettlementType ?? null);
    }
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
          settlementName={settlementName === undefined ? undefined : currentSettlementName}
          settlementCoordinates={
            settlementCoordinates === undefined ? undefined : currentSettlementCoordinates
          }
          settlementType={currentSettlementType}
          settlementTypes={settlementTypes}
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
  onSaved: (
    body: PostDocument | null,
    images: PostImage[],
    settlementName?: string,
    settlementCoordinates?: string,
    settlementType?: string | null,
  ) => void;
  onCancel: () => void;
  /** When set, the form also saves the ordered image set and mounts the picker. */
  imageSupport?: boolean;
  initialImages?: PostImage[];
  /** Enables the settlement-name field and includes its value in the PATCH. */
  settlementName?: string;
  /** Enables settlement coordinates/type fields and includes them in the PATCH. */
  settlementCoordinates?: string;
  settlementType?: string | null;
  settlementTypes?: string[];
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
  settlementName,
  settlementCoordinates,
  settlementType,
  settlementTypes = [],
}: RichPageContentFormProps) {
  const editor = usePostDocumentEditor(initialBody ?? "", editorLabel);
  const nameId = useId();
  const nameRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState(settlementName ?? "");
  const coordinatesId = useId();
  const typeId = useId();
  const coordinatesRef = useRef<HTMLInputElement>(null);
  const typeRef = useRef<HTMLSelectElement>(null);
  const [coordinates, setCoordinates] = useState(settlementCoordinates ?? "");
  const [typeValue, setTypeValue] = useState(settlementType ?? "");
  const metadataEnabled = settlementCoordinates !== undefined;
  const typeOptions =
    settlementType && !settlementTypes.includes(settlementType)
      ? [settlementType, ...settlementTypes]
      : settlementTypes;
  const richTextFieldRef = useRef<RichTextFieldHandle>(null);
  const imageUploaderRef = useRef<PostImageUploaderHandle>(null);
  const [saving, setSaving] = useState(false);
  /** True while the images are uploading or one of them failed. */
  const [imagesBlocked, setImagesBlocked] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (settlementName !== undefined) {
      nameRef.current?.focus();
      return;
    }
    editor?.commands.focus();
  }, [editor, settlementName]);

  async function save() {
    if (!editor || saving) return;
    const nextName = settlementName === undefined ? undefined : name.trim();
    if (nextName !== undefined && !nextName) {
      setError("Название населённого пункта не должно быть пустым.");
      nameRef.current?.focus();
      return;
    }
    if (nextName !== undefined && nextName.length > SETTLEMENT_NAME_MAX_LENGTH) {
      setError(
        `Название населённого пункта должно быть не длиннее ${SETTLEMENT_NAME_MAX_LENGTH} символов.`,
      );
      nameRef.current?.focus();
      return;
    }
    const parsedCoordinates = metadataEnabled ? parseCoordinates(coordinates) : null;
    if (metadataEnabled && !parsedCoordinates) {
      setError("Укажите координаты в формате «широта, долгота», например 56.1234, 40.5678.");
      coordinatesRef.current?.focus();
      return;
    }
    const nextType = metadataEnabled && typeValue ? typeValue : undefined;
    if (metadataEnabled && settlementType !== null && !nextType) {
      setError("Выберите тип населённого пункта.");
      typeRef.current?.focus();
      return;
    }
    const imageUploader = imageSupport ? imageUploaderRef.current : null;
    if (imageUploader?.isBlocked()) {
      setError("Дождитесь завершения загрузки изображений или удалите файлы с ошибкой.");
      return;
    }
    const imageIds = imageUploader?.getImageIds() ?? [];
    const referenceIsEmpty = !editor.getText().trim() && imageIds.length === 0;
    const metadataOnlySave =
      nextName !== undefined &&
      !documentHasVisibleText(initialBody) &&
      initialImages.length === 0 &&
      referenceIsEmpty;
    // A metadata-only save is permitted only for a record whose reference was
    // already empty when this form opened. Existing content is never cleared
    // by omitting the reference fields.
    if (referenceIsEmpty && !metadataOnlySave) {
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
        body: JSON.stringify(
          metadataOnlySave
            ? {
                name: nextName,
                ...(parsedCoordinates
                  ? { coordinates: `${parsedCoordinates.latitude}, ${parsedCoordinates.longitude}` }
                  : {}),
                ...(nextType === undefined ? {} : { type: nextType }),
              }
            : {
                ...(nextName === undefined ? {} : { name: nextName }),
                ...(parsedCoordinates
                  ? { coordinates: `${parsedCoordinates.latitude}, ${parsedCoordinates.longitude}` }
                  : {}),
                ...(nextType === undefined ? {} : { type: nextType }),
                body: editor.getJSON(),
                // The ordered final set is sent every time: kept images in order plus
                // new uploads. Omission would leave stored images untouched instead.
                ...(imageSupport ? { imageIds } : {}),
              },
        ),
      });
      if (!response.ok) {
        setError(await errorMessage(response, saveErrorMessage));
        return;
      }

      const data = (await response.json()) as {
        body?: PostDocument | null;
        images?: PostImage[];
        name?: unknown;
        coordinates?: unknown;
        type?: unknown;
      };
      const nextImages = Array.isArray(data.images) ? data.images : [];
      if (!data.body && (!imageSupport || nextImages.length === 0) && !metadataOnlySave) {
        setError("Сервер вернул некорректный ответ. Попробуйте ещё раз.");
        return;
      }
      const savedName =
        nextName === undefined
          ? undefined
          : typeof data.name === "string" && data.name.trim()
            ? data.name
            : nextName;
      let savedCoordinates: string | undefined;
      let savedType: string | null | undefined;
      if (metadataEnabled) {
        if (
          typeof data.coordinates !== "string" ||
          !parseCoordinates(data.coordinates) ||
          (typeof data.type !== "string" && data.type !== null)
        ) {
          setError("Сервер вернул некорректный ответ. Попробуйте ещё раз.");
          return;
        }
        savedCoordinates = data.coordinates;
        savedType = data.type;
      }
      // The stored record now owns the new uploads: never clean them up as pending.
      imageUploaderRef.current?.claimAll();
      onSaved(data.body ?? null, nextImages, savedName, savedCoordinates, savedType);
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
      {settlementName !== undefined ? (
        <>
          <label className={styles.bodyLabel} htmlFor={nameId}>
            Название населённого пункта
          </label>
          <input
            ref={nameRef}
            id={nameId}
            name="name"
            className={styles.nameInput}
            value={name}
            maxLength={SETTLEMENT_NAME_MAX_LENGTH}
            autoComplete="off"
            spellCheck={false}
            required
            disabled={saving}
            onChange={(event) => {
              setName(event.target.value);
              setError("");
            }}
          />
        </>
      ) : null}

      {metadataEnabled ? (
        <div className={styles.metadataFields}>
          <label className={styles.metadataField} htmlFor={coordinatesId}>
            <span className={styles.bodyLabel}>Координаты</span>
            <input
              ref={coordinatesRef}
              id={coordinatesId}
              name="coordinates"
              className={styles.nameInput}
              value={coordinates}
              inputMode="decimal"
              autoComplete="off"
              spellCheck={false}
              required
              disabled={saving}
              onChange={(event) => {
                setCoordinates(event.target.value);
                setError("");
              }}
            />
          </label>
          <label className={styles.metadataField} htmlFor={typeId}>
            <span className={styles.bodyLabel}>Тип населённого пункта</span>
            <select
              ref={typeRef}
              id={typeId}
              name="type"
              className={styles.nameInput}
              value={typeValue}
              required={settlementType !== null}
              disabled={saving}
              onChange={(event) => {
                setTypeValue(event.target.value);
                setError("");
              }}
            >
              {settlementType === null ? <option value="">Не указан</option> : null}
              {typeOptions.map((option) => (
                <option key={option} value={option}>
                  {option}
                </option>
              ))}
            </select>
          </label>
        </div>
      ) : null}

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
