"use client";

import { useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import type { SiteSettingKind, SiteSettings } from "@/lib/site-settings";
import styles from "./SettingsEditor.module.css";

type SettingsEditorProps = {
  initialSettings: SiteSettings;
};

const SETTINGS_LISTS: {
  kind: SiteSettingKind;
  title: string;
  description: string;
  label: string;
  placeholder: string;
}[] = [
  {
    kind: "categories",
    title: "Категории сообщений",
    description: "Выбор категории при публикации сообщения в губернии.",
    label: "Новая категория",
    placeholder: "Например: Перепись",
  },
  {
    kind: "settlementTypes",
    title: "Типы населённых пунктов",
    description: "Выбор типа при создании населённого пункта.",
    label: "Новый тип",
    placeholder: "Например: Посёлок",
  },
];

/**
 * Adds, renames, and removes names in the two persisted settings lists. The
 * endpoint owns authorization, normalization, and queue-serialized atomic
 * writes; the canonical settings come back in every response so the rendered
 * lists stay in sync with the stored file. Edits only change the options
 * offered for future creations — existing publications and settlements keep
 * their historical values.
 */
export default function SettingsEditor({ initialSettings }: SettingsEditorProps) {
  const [settings, setSettings] = useState(initialSettings);
  // One mutation at a time across both lists: responses carry whole settings,
  // so overlapping writes could otherwise render a stale list.
  const [busy, setBusy] = useState(false);

  return (
    <div className={styles.editor}>
      {SETTINGS_LISTS.map((list) => (
        <SettingsList
          key={list.kind}
          kind={list.kind}
          title={list.title}
          description={list.description}
          label={list.label}
          placeholder={list.placeholder}
          values={settings[list.kind]}
          onSaved={setSettings}
          busy={busy}
          onBusyChange={setBusy}
        />
      ))}
    </div>
  );
}

type SettingsListProps = {
  kind: SiteSettingKind;
  title: string;
  description: string;
  label: string;
  placeholder: string;
  values: string[];
  onSaved: (settings: SiteSettings) => void;
  busy: boolean;
  onBusyChange: (busy: boolean) => void;
};

type EditingRow = {
  originalName: string;
  value: string;
};

type PendingRow = {
  originalName: string;
  action: "rename" | "delete";
};

function SettingsList({
  kind,
  title,
  description,
  label,
  placeholder,
  values,
  onSaved,
  busy,
  onBusyChange,
}: SettingsListProps) {
  const router = useRouter();
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [editing, setEditing] = useState<EditingRow | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [pendingRow, setPendingRow] = useState<PendingRow | null>(null);
  const inputId = `settings-${kind}`;
  const sectionId = `settings-section-${kind}`;
  const busyNow = busy || saving;

  async function requestSettings(
    method: "POST" | "PATCH" | "DELETE",
    body: Record<string, string>,
  ): Promise<SiteSettings | null> {
    try {
      const response = await fetch("/api/nastroyki", {
        method,
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify(body),
      });
      const payload = (await response.json().catch(() => null)) as
        | { settings?: SiteSettings; error?: string }
        | null;
      if (!response.ok || !payload?.settings) {
        setError(payload?.error ?? "Не удалось сохранить. Попробуйте ещё раз.");
        return null;
      }
      return payload.settings;
    } catch {
      setError("Не удалось сохранить. Попробуйте ещё раз.");
      return null;
    }
  }

  async function handleAdd(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busyNow) return;

    if (!name.trim()) {
      setStatus("");
      setError("Введите название.");
      return;
    }

    setSaving(true);
    onBusyChange(true);
    setError("");
    setStatus("");
    try {
      const settings = await requestSettings("POST", { kind, name });
      if (!settings) return;
      onSaved(settings);
      setName("");
      setStatus("Добавлено.");
      router.refresh();
    } finally {
      setSaving(false);
      onBusyChange(false);
    }
  }

  async function handleRename(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!editing || busyNow) return;

    if (!editing.value.trim()) {
      setStatus("");
      setError("Введите название.");
      return;
    }

    const originalName = editing.originalName;
    setPendingRow({ originalName, action: "rename" });
    onBusyChange(true);
    setError("");
    setStatus("");
    try {
      const settings = await requestSettings("PATCH", { kind, originalName, name: editing.value });
      // A failed rename keeps the row editable with the entered text intact.
      if (!settings) return;
      onSaved(settings);
      setEditing(null);
      setStatus("Переименовано.");
      router.refresh();
    } finally {
      setPendingRow(null);
      onBusyChange(false);
    }
  }

  async function handleDelete(originalName: string) {
    if (busyNow) return;

    setPendingRow({ originalName, action: "delete" });
    onBusyChange(true);
    setError("");
    setStatus("");
    try {
      const settings = await requestSettings("DELETE", { kind, name: originalName });
      // A failed delete keeps the confirmation open so it can be retried.
      if (!settings) return;
      onSaved(settings);
      setConfirming(null);
      setStatus("Удалено.");
      router.refresh();
    } finally {
      setPendingRow(null);
      onBusyChange(false);
    }
  }

  function beginEdit(value: string) {
    if (busyNow) return;
    setEditing({ originalName: value, value });
    setConfirming(null);
    setError("");
    setStatus("");
  }

  function cancelEdit() {
    setEditing(null);
    setError("");
  }

  function beginDelete(value: string) {
    if (busyNow) return;
    setEditing(null);
    setConfirming(value);
    setError("");
    setStatus("");
  }

  function cancelDelete() {
    setConfirming(null);
    setError("");
  }

  return (
    <section className={styles.section} aria-labelledby={sectionId}>
      <h2 className={styles.sectionTitle} id={sectionId}>
        {title}
      </h2>
      <p className={styles.sectionText}>{description}</p>

      {values.length === 0 ? (
        <p className={styles.empty}>Список пуст — добавьте первое название.</p>
      ) : (
        <ul className={styles.list}>
          {values.map((value) => {
            const isEditing = editing?.originalName === value;
            const isConfirming = confirming === value;
            const isPendingRename = pendingRow?.originalName === value && pendingRow.action === "rename";
            const isPendingDelete = pendingRow?.originalName === value && pendingRow.action === "delete";
            return (
              <li className={styles.item} key={value}>
                {isEditing ? (
                  <form className={styles.editForm} onSubmit={handleRename} noValidate>
                    <input
                      className={styles.editInput}
                      type="text"
                      value={editing.value}
                      aria-label={`Новое название вместо «${value}»`}
                      autoComplete="off"
                      autoFocus
                      disabled={pendingRow !== null}
                      onChange={(event) => {
                        setEditing({ originalName: value, value: event.target.value });
                        if (error) setError("");
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Escape") {
                          event.preventDefault();
                          cancelEdit();
                        }
                      }}
                    />
                    <button
                      className={styles.editSave}
                      type="submit"
                      disabled={busyNow || pendingRow !== null}
                    >
                      {isPendingRename ? "Сохраняем…" : "Сохранить"}
                    </button>
                    <button
                      className={styles.editCancel}
                      type="button"
                      onClick={cancelEdit}
                      disabled={busyNow || pendingRow !== null}
                    >
                      Отмена
                    </button>
                  </form>
                ) : (
                  <div className={styles.itemRow}>
                    <span className={styles.itemName}>{value}</span>
                    <span className={styles.itemActions}>
                      <button
                        type="button"
                        className={styles.rowButton}
                        onClick={() => beginEdit(value)}
                        disabled={busyNow}
                      >
                        Редактировать
                      </button>
                      <button
                        type="button"
                        className={styles.removeButton}
                        onClick={() => beginDelete(value)}
                        disabled={busyNow}
                      >
                        Удалить
                      </button>
                    </span>
                  </div>
                )}

                {isConfirming && !isEditing ? (
                  <div
                    className={styles.confirm}
                    role="group"
                    aria-label={`Подтверждение удаления «${value}»`}
                    onKeyDown={(event) => {
                      if (event.key === "Escape") {
                        event.preventDefault();
                        cancelDelete();
                      }
                    }}
                  >
                    <p className={styles.confirmText}>
                      Удалить «{value}»? Название исчезнет из списка и больше не будет предлагаться
                      при создании материалов. Уже созданные материалы сохранят это значение.
                    </p>
                    <div className={styles.confirmActions}>
                      <button
                        type="button"
                        className={styles.confirmDelete}
                        onClick={() => void handleDelete(value)}
                        disabled={busyNow}
                      >
                        {isPendingDelete ? "Удаляем…" : "Удалить"}
                      </button>
                      <button
                        type="button"
                        className={styles.confirmCancel}
                        onClick={cancelDelete}
                        disabled={busyNow}
                      >
                        Отмена
                      </button>
                    </div>
                  </div>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}

      <form className={styles.form} onSubmit={handleAdd} noValidate>
        <div className={styles.field}>
          <label className={styles.fieldLabel} htmlFor={inputId}>
            {label}
          </label>
          <input
            className={styles.input}
            id={inputId}
            name="name"
            type="text"
            value={name}
            placeholder={placeholder}
            autoComplete="off"
            onChange={(event) => {
              setName(event.target.value);
              if (error) setError("");
              if (status) setStatus("");
            }}
          />
        </div>
        <button className={styles.addButton} type="submit" disabled={busyNow}>
          {saving ? "Сохраняем…" : "Добавить"}
        </button>
      </form>

      {error ? (
        <p className={styles.error} role="alert">
          {error}
        </p>
      ) : status ? (
        <p className={styles.status} role="status">
          {status}
        </p>
      ) : null}
    </section>
  );
}
