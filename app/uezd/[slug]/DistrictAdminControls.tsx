"use client";

import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useRouter } from "next/navigation";
import PostImageUploader, {
  type PostImageUploaderHandle,
} from "@/app/guberniya/[slug]/PostImageUploader";
import RichTextField, {
  usePostDocumentEditor,
  type RichTextFieldHandle,
} from "@/components/RichTextField";
import {
  DISTRICT_URL_PREFIX,
  DISTRICT_URL_SLUG_MAX_LENGTH,
  districtUrlForSlug,
  isValidDistrictSlug,
} from "@/lib/district-url";
import type { PostImage } from "@/lib/gubernia-publications";
import type { PostDocument } from "@/lib/post-content";
import styles from "./DistrictAdminControls.module.css";

const DISTRICT_NAME_MAX_LENGTH = 200;

type Props = {
  guberniaId: string;
  guberniaName: string;
  districtId: string;
  districtName: string;
  url: string;
  description: PostDocument | null;
  images: PostImage[];
};

type UpdateResponse = {
  url?: unknown;
};

async function errorMessage(response: Response, fallback: string) {
  try {
    const data = (await response.json()) as { error?: unknown };
    if (typeof data.error === "string" && data.error.trim()) return data.error;
  } catch {
    /* Non-JSON error body: use the stable local message. */
  }
  return fallback;
}

export default function DistrictAdminControls(props: Props) {
  const router = useRouter();
  const [editing, setEditing] = useState(false);
  const [saved, setSaved] = useState(false);
  const editTriggerRef = useRef<HTMLButtonElement>(null);

  function cancelEdit() {
    setSaved(false);
    setEditing(false);
    requestAnimationFrame(() => editTriggerRef.current?.focus());
  }

  function handleSaved(nextUrl: string) {
    if (nextUrl !== props.url) {
      router.push(nextUrl);
      return;
    }
    setEditing(false);
    setSaved(true);
    router.refresh();
    requestAnimationFrame(() => editTriggerRef.current?.focus());
  }

  return (
    <section className={styles.panel} aria-label="Управление уездом">
      {editing ? (
        <DistrictEditForm {...props} onSaved={handleSaved} onCancel={cancelEdit} />
      ) : (
        <>
          {saved ? <p className={styles.status} role="status">Сохранено.</p> : null}
          <button
            ref={editTriggerRef}
            className={styles.editButton}
            type="button"
            onClick={() => {
              setSaved(false);
              setEditing(true);
            }}
          >
            Редактировать уезд
          </button>
        </>
      )}
    </section>
  );
}

type EditProps = Props & {
  onSaved: (url: string) => void;
  onCancel: () => void;
};

function DistrictEditForm({
  guberniaId,
  guberniaName,
  districtId,
  districtName,
  url,
  description,
  images,
  onSaved,
  onCancel,
}: EditProps) {
  const initialSlug = url.startsWith(DISTRICT_URL_PREFIX)
    ? url.slice(DISTRICT_URL_PREFIX.length)
    : "";
  const editor = usePostDocumentEditor(description ?? "", "Справочные сведения");
  const richTextFieldRef = useRef<RichTextFieldHandle>(null);
  const imageUploaderRef = useRef<PostImageUploaderHandle>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const slugRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState(districtName);
  const [slug, setSlug] = useState(initialSlug);
  const [saving, setSaving] = useState(false);
  const [imagesBlocked, setImagesBlocked] = useState(false);
  const [saveError, setSaveError] = useState("");

  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  async function save() {
    if (!editor || saving) return;
    const uploader = imageUploaderRef.current;
    if (uploader?.isBlocked()) {
      setSaveError("Дождитесь завершения загрузки изображений или удалите файлы с ошибкой.");
      return;
    }

    const nextName = name.trim();
    if (!nextName) {
      setSaveError("Название уезда не может быть пустым.");
      nameRef.current?.focus();
      return;
    }
    if (nextName.length > DISTRICT_NAME_MAX_LENGTH) {
      setSaveError(`Название уезда должно быть не длиннее ${DISTRICT_NAME_MAX_LENGTH} символов.`);
      nameRef.current?.focus();
      return;
    }

    const nextSlug = slug.trim();
    if (!nextSlug) {
      setSaveError("Адрес страницы не может быть пустым.");
      slugRef.current?.focus();
      return;
    }
    if (nextSlug.length > DISTRICT_URL_SLUG_MAX_LENGTH) {
      setSaveError(`Адрес страницы должен быть не длиннее ${DISTRICT_URL_SLUG_MAX_LENGTH} символов.`);
      slugRef.current?.focus();
      return;
    }
    if (!isValidDistrictSlug(nextSlug)) {
      setSaveError("Адрес должен содержать только строчные латинские буквы, цифры и одиночные дефисы.");
      slugRef.current?.focus();
      return;
    }

    const nextUrl = districtUrlForSlug(nextSlug);
    const nextDescription = editor.getText().trim() ? editor.getJSON() : null;
    const imageIds = uploader?.getImageIds() ?? [];
    setSaving(true);
    setSaveError("");
    uploader?.setSaveInFlight(true);
    try {
      const response = await fetch(
        `/api/gubernias/${encodeURIComponent(guberniaId)}/districts/${encodeURIComponent(districtId)}`,
        {
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
          credentials: "same-origin",
          body: JSON.stringify({
            name: nextName,
            url: nextUrl,
            description: nextDescription,
            imageIds,
          }),
        },
      );
      if (!response.ok) {
        setSaveError(await errorMessage(response, "Не удалось сохранить изменения. Попробуйте ещё раз."));
        return;
      }

      let data: UpdateResponse = {};
      try {
        data = (await response.json()) as UpdateResponse;
      } catch {
        /* The submitted canonical URL is still known if the body is unreadable. */
      }
      imageUploaderRef.current?.claimAll();
      onSaved(typeof data.url === "string" && data.url.startsWith(DISTRICT_URL_PREFIX)
        ? data.url
        : nextUrl);
    } catch {
      setSaveError("Не удалось связаться с сервером.");
    } finally {
      uploader?.setSaveInFlight(false);
      setSaving(false);
    }
  }

  function cancel() {
    if (saving) return;
    imageUploaderRef.current?.cleanupPending();
    onCancel();
  }

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void save();
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
    if (editor && target instanceof Node && editor.view.dom.contains(target)) return;
    event.preventDefault();
    cancel();
  }

  return (
    <form className={styles.form} onSubmit={handleSubmit} onKeyDown={handleKeyDown}>
      <dl className={styles.context}>
        <div><dt>Губерния</dt><dd>{guberniaName}</dd></div>
        <div><dt>Уезд</dt><dd>{districtName}</dd></div>
      </dl>

      <label htmlFor="district-name">Название уезда</label>
      <input
        ref={nameRef}
        id="district-name"
        name="name"
        className={styles.textInput}
        value={name}
        maxLength={DISTRICT_NAME_MAX_LENGTH}
        autoComplete="off"
        spellCheck={false}
        required
        onChange={(event) => {
          setName(event.target.value);
          setSaveError("");
        }}
      />

      <label htmlFor="district-slug">Адрес страницы</label>
      <div className={styles.addressField}>
        <span className={styles.addressPrefix}>{DISTRICT_URL_PREFIX}</span>
        <input
          ref={slugRef}
          id="district-slug"
          name="slug"
          className={styles.addressInput}
          value={slug}
          maxLength={DISTRICT_URL_SLUG_MAX_LENGTH}
          placeholder="aleksinskiy"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          required
          onChange={(event) => {
            setSlug(event.target.value);
            setSaveError("");
          }}
        />
      </div>
      <p className={styles.hint}>Локальный адрес латиницей: строчные буквы, цифры и дефисы.</p>

      <span className={styles.descriptionLabel}>Справочные сведения</span>
      <RichTextField
        ref={richTextFieldRef}
        editor={editor}
        hint="Текст необязателен. Enter — новый абзац, Shift+Enter — перенос строки внутри абзаца."
      />
      <PostImageUploader
        ref={imageUploaderRef}
        initialImages={images}
        disabled={saving}
        onBlockedChange={setImagesBlocked}
      />

      {saveError ? <p className={styles.error} role="alert">{saveError}</p> : null}
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
