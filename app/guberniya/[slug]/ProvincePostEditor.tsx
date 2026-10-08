"use client";

import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { useRouter } from "next/navigation";
import RichTextField, {
  usePostDocumentEditor,
  type RichTextFieldHandle,
} from "@/components/RichTextField";
import type { GuberniaPost, PublishedOption, Settlement } from "@/lib/gubernia-publications";
import { useDistrictOptions } from "./useDistrictOptions";
import styles from "./ProvincePostEditor.module.css";

const TITLE_MAX_LENGTH = 1000;
const MAX_YEAR_LENGTH = 100;
const MAX_ARCHIVE_REFERENCE_LENGTH = 300;

type ProvincePostEditorProps = {
  guberniaId: string;
  provinces: PublishedOption[];
  settlements: Settlement[];
  categories: string[];
  post?: GuberniaPost;
  /**
   * Settlement preselected for a new post and restored after saving or
   * cancelling; ignored while editing an existing post.
   */
  defaultSettlementId?: string;
  /** Called once the editor is finished: the post was saved or editing stopped. */
  onDone?: () => void;
};

export default function ProvincePostEditor({
  guberniaId,
  provinces,
  settlements,
  categories,
  post,
  defaultSettlementId,
  onDone,
}: ProvincePostEditorProps) {
  const router = useRouter();
  const headingId = useId();
  const titleId = useId();
  const provinceId = useId();
  const districtId = useId();
  const settlementId = useId();
  const yearId = useId();
  const archiveReferenceId = useId();
  const categoryId = useId();

  const isEditing = post !== undefined;
  const defaultSettlement = useMemo(
    () =>
      isEditing || !defaultSettlementId
        ? null
        : settlements.find(
            (settlement) =>
              settlement.id === defaultSettlementId && settlement.guberniaId === guberniaId,
          ) ?? null,
    [defaultSettlementId, guberniaId, isEditing, settlements],
  );
  const defaultDistrictId = post?.uyezdId ?? defaultSettlement?.uyezdId ?? "";
  const [title, setTitle] = useState(post?.title ?? "");
  const [category, setCategory] = useState<string>(post?.category ?? "");
  const [selectedProvinceId, setSelectedProvinceId] = useState(
    defaultSettlement?.guberniaId ?? guberniaId,
  );
  const [selectedDistrictId, setSelectedDistrictId] = useState(defaultDistrictId);
  const [selectedSettlementId, setSelectedSettlementId] = useState(
    post?.settlementId ?? defaultSettlement?.id ?? "",
  );
  const [year, setYear] = useState(post?.year ?? "");
  const [archiveReference, setArchiveReference] = useState(post?.archiveReference ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const titleRef = useRef<HTMLInputElement>(null);
  const categoryRef = useRef<HTMLSelectElement>(null);
  const provinceRef = useRef<HTMLSelectElement>(null);
  const richTextFieldRef = useRef<RichTextFieldHandle>(null);

  const { districts, loading: districtsLoading, error: districtsError } =
    useDistrictOptions(selectedProvinceId);
  const filteredSettlements = useMemo(
    () =>
      settlements
        .filter(
          (settlement) =>
            (!selectedProvinceId || settlement.guberniaId === selectedProvinceId) &&
            (!selectedDistrictId || settlement.uyezdId === selectedDistrictId),
        )
        .sort((left, right) => left.name.localeCompare(right.name, "ru")),
    [selectedDistrictId, selectedProvinceId, settlements],
  );
  const categoryOptions = useMemo(
    () => post?.category && !categories.includes(post.category)
      ? [...categories, post.category]
      : categories,
    [categories, post?.category],
  );

  const editor = usePostDocumentEditor(post?.body ?? "", "Тело сообщения");

  useEffect(() => {
    if (!selectedSettlementId) return;
    if (filteredSettlements.some((settlement) => settlement.id === selectedSettlementId)) return;
    // The current filters no longer contain it: drop the id instead of sending a foreign one.
    setSelectedSettlementId("");
  }, [filteredSettlements, selectedSettlementId]);

  function handleProvinceChange(value: string) {
    setSelectedProvinceId(value);
    setSelectedDistrictId("");
    setSelectedSettlementId("");
    setStatus("");
  }

  function handleSettlementChange(value: string) {
    setStatus("");
    if (!value) {
      setSelectedSettlementId("");
      return;
    }
    const settlement = settlements.find((item) => item.id === value);
    if (!settlement) {
      setSelectedSettlementId("");
      return;
    }
    // Picking a settlement restores the owning province and district of the cascading fields.
    setSelectedSettlementId(settlement.id);
    setSelectedProvinceId(settlement.guberniaId);
    setSelectedDistrictId(settlement.uyezdId);
  }

  function cancel() {
    if (saving) return;
    setError("");
    setStatus("");
    if (richTextFieldRef.current?.isLinkFieldOpen()) richTextFieldRef.current.closeLinkField();
    if (isEditing) {
      // The parent owns the editor in edit mode; it closes it on this signal.
      onDone?.();
      return;
    }
    setTitle("");
    setCategory("");
    setSelectedProvinceId(guberniaId);
    setSelectedDistrictId(defaultDistrictId);
    setSelectedSettlementId(defaultSettlement?.id ?? "");
    setYear("");
    setArchiveReference("");
    editor?.commands.clearContent();
    if (onDone) {
      // Creating inside the panel: cancel drops the draft and closes the composer.
      onDone();
      return;
    }
    titleRef.current?.focus();
  }

  async function save() {
    if (!editor || saving) return;
    const nextTitle = title.trim();
    if (!nextTitle) {
      setStatus("");
      setError("Введите заголовок сообщения.");
      titleRef.current?.focus();
      return;
    }
    if (!editor.getText().trim()) {
      setStatus("");
      setError("Добавьте текст сообщения.");
      editor.chain().focus().run();
      return;
    }
    if (!category) {
      setStatus("");
      setError("Выберите категорию сообщения.");
      categoryRef.current?.focus();
      return;
    }
    if (!selectedProvinceId) {
      setStatus("");
      setError("Выберите губернию: сообщение хранится на странице опубликованной губернии.");
      provinceRef.current?.focus();
      return;
    }

    let nextDistrictId = selectedDistrictId;
    if (selectedSettlementId) {
      const settlement = settlements.find((item) => item.id === selectedSettlementId);
      if (!settlement || settlement.guberniaId !== selectedProvinceId) {
        setStatus("");
        setError("Выбранный населённый пункт не относится к этой губернии. Выберите его заново.");
        setSelectedSettlementId("");
        return;
      }
      // A settlement without an explicit district stores the district it belongs to.
      if (!nextDistrictId) nextDistrictId = settlement.uyezdId;
    }

    const postId = post?.id;
    const targetSlug = provinces.find((province) => province.id === selectedProvinceId)?.slug;
    setSaving(true);
    setError("");
    setStatus("");
    try {
      const response = await fetch(
        postId
          ? `/api/gubernias/${encodeURIComponent(guberniaId)}/posts/${encodeURIComponent(postId)}`
          : `/api/gubernias/${encodeURIComponent(selectedProvinceId)}/posts`,
        {
          method: postId ? "PATCH" : "POST",
          headers: { "Content-Type": "application/json" },
          credentials: "same-origin",
          body: JSON.stringify({
            title: nextTitle,
            body: editor.getJSON(),
            category,
            uyezdId: nextDistrictId || null,
            settlementId: selectedSettlementId || null,
            year: year.trim(),
            archiveReference: archiveReference.trim(),
            ...(postId ? { targetGuberniaId: selectedProvinceId } : {}),
          }),
        },
      );
      if (!response.ok) {
        setError(
          await errorMessage(
            response,
            postId
              ? "Не удалось сохранить изменения. Попробуйте ещё раз."
              : "Не удалось отправить сообщение. Попробуйте ещё раз.",
          ),
        );
        return;
      }

      if (selectedProvinceId !== guberniaId && targetSlug) {
        // The post now lives on another province page: follow it there.
        if (postId) onDone?.();
        router.push(`/guberniya/${targetSlug}`);
        return;
      }
      if (postId) {
        onDone?.();
      } else {
        // The post is stored already, so the form goes back to a blank draft
        // that still points at the settlement the page was opened from.
        setTitle("");
        setCategory("");
        setSelectedDistrictId(defaultDistrictId);
        setSelectedSettlementId(defaultSettlement?.id ?? "");
        setYear("");
        setArchiveReference("");
        editor.commands.clearContent();
        setStatus("Сообщение опубликовано.");
        titleRef.current?.focus();
      }
      router.refresh();
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

  function handleFormKeyDown(event: KeyboardEvent<HTMLFormElement>) {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void save();
      return;
    }
    if (event.key !== "Escape") return;
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
    <section className={styles.panel} aria-labelledby={headingId}>
      <h2 id={headingId} className={styles.heading}>
        {isEditing ? "Редактирование сообщения" : "Новое сообщение"}
      </h2>
      <form className={styles.form} onSubmit={handleSubmit} onKeyDown={handleFormKeyDown} noValidate>
        <label htmlFor={titleId}>Заголовок сообщения</label>
        <input
          ref={titleRef}
          id={titleId}
          name="title"
          className={styles.titleInput}
          value={title}
          maxLength={TITLE_MAX_LENGTH}
          autoComplete="off"
          spellCheck={false}
          required
          onChange={(event) => {
            setTitle(event.target.value);
            setStatus("");
          }}
        />

        <span className={styles.bodyLabel}>Тело сообщения</span>
        <RichTextField
          ref={richTextFieldRef}
          editor={editor}
          hint="Enter — новый абзац, Shift+Enter — перенос строки внутри абзаца."
        />

        <div className={styles.meta}>
          <div className={styles.metaField}>
            <label htmlFor={categoryId}>Категория</label>
            <select
              ref={categoryRef}
              id={categoryId}
              name="category"
              className={styles.metaSelect}
              value={category}
              required
              onChange={(event) => {
                setCategory(event.target.value);
                setStatus("");
              }}
            >
              <option value="">— Не выбрана —</option>
              {categoryOptions.map((option) => (
                <option key={option} value={option}>
                  {option}
                </option>
              ))}
            </select>
          </div>

          <div className={styles.metaField}>
            <label htmlFor={provinceId}>Губерния</label>
            <select
              ref={provinceRef}
              id={provinceId}
              name="guberniaId"
              className={styles.metaSelect}
              value={selectedProvinceId}
              onChange={(event) => handleProvinceChange(event.target.value)}
            >
              <option value="">— Не выбрана —</option>
              {provinces.map((province) => (
                <option key={province.id} value={province.id}>
                  {province.name}
                </option>
              ))}
            </select>
          </div>

          <div className={styles.metaField}>
            <label htmlFor={districtId}>Уезд</label>
            <select
              id={districtId}
              name="uyezdId"
              className={styles.metaSelect}
              value={selectedDistrictId}
              disabled={!selectedProvinceId || districtsLoading}
              onChange={(event) => {
                const nextDistrictId = event.target.value;
                setSelectedDistrictId(nextDistrictId);
                setSelectedSettlementId((previous) =>
                  previous && settlements.find((item) => item.id === previous)?.uyezdId === nextDistrictId
                    ? previous
                    : "",
                );
                setStatus("");
              }}
            >
              <option value="">{districtsLoading ? "Загружаем…" : "— Не указан —"}</option>
              {selectedSettlementId && selectedDistrictId && !districts.some((district) => district.id === selectedDistrictId) ? (
                <option value={selectedDistrictId}>
                  {districtsLoading ? "Уезд населённого пункта — загружаем…" : "Уезд населённого пункта"}
                </option>
              ) : null}
              {districts.map((district) => (
                <option key={district.id} value={district.id}>
                  {district.name}
                </option>
              ))}
            </select>

          </div>

          <div className={styles.metaField}>
            <label htmlFor={settlementId}>Населённый пункт</label>
            <select
              id={settlementId}
              name="settlementId"
              className={styles.metaSelect}
              value={selectedSettlementId}
              onChange={(event) => handleSettlementChange(event.target.value)}
            >
              <option value="">— Не указан —</option>
              {filteredSettlements.map((settlement) => (
                <option key={settlement.id} value={settlement.id}>
                  {settlement.name}
                </option>
              ))}
            </select>
          </div>

          <div className={styles.metaField}>
            <label htmlFor={yearId}>Год</label>
            <input
              id={yearId}
              name="year"
              className={styles.metaInput}
              value={year}
              maxLength={MAX_YEAR_LENGTH}
              autoComplete="off"
              spellCheck={false}
              placeholder="Например, 1897"
              onChange={(event) => {
                setYear(event.target.value);
                setStatus("");
              }}
            />
          </div>

          <div className={styles.metaField}>
            <label htmlFor={archiveReferenceId}>Архивный шифр</label>
            <input
              id={archiveReferenceId}
              name="archiveReference"
              className={styles.metaInput}
              value={archiveReference}
              maxLength={MAX_ARCHIVE_REFERENCE_LENGTH}
              autoComplete="off"
              spellCheck={false}
              placeholder="Например, ГАВО. Ф. 32. Оп. 1. Д. 5"
              onChange={(event) => {
                setArchiveReference(event.target.value);
                setStatus("");
              }}
            />
          </div>
        </div>

        {selectedSettlementId ? (
          <p className={styles.hint}>
            Уезд выбран автоматически по населённому пункту. Если выбрать другой уезд, населённый пункт сбросится.
          </p>
        ) : null}
        {districtsError ? <p className={styles.hint}>{districtsError}</p> : null}
        <p className={styles.hint}>
          Если снять губернию и уезд, в списке населённых пунктов появятся все опубликованные пункты.
        </p>

        {error ? (
          <p className={styles.error} role="alert">
            {error}
          </p>
        ) : null}
        {status ? (
          <p className={styles.status} role="status">
            {status}
          </p>
        ) : null}

        <div className={styles.actions}>
          <button className={styles.primaryButton} type="submit" disabled={saving || !editor}>
            {saving ? (isEditing ? "Сохраняем…" : "Отправляем…") : isEditing ? "Сохранить" : "Отправить"}
          </button>
          <button className={styles.cancelButton} type="button" onClick={cancel} disabled={saving}>
            Отмена
          </button>
        </div>
      </form>
    </section>
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
