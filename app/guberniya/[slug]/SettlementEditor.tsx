"use client";

import {
  useId,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { useRouter } from "next/navigation";
import type { PublishedOption } from "@/lib/gubernia-publications";
import {
  SETTLEMENT_URL_PREFIX,
  SETTLEMENT_URL_SLUG_MAX_LENGTH,
  isValidSettlementSlug,
  settlementUrlForSlug,
} from "@/lib/settlement-url";
import { useDistrictOptions } from "./useDistrictOptions";
import styles from "./SettlementEditor.module.css";

const NAME_MAX_LENGTH = 200;

type SettlementEditorProps = {
  guberniaId: string;
  provinces: PublishedOption[];
  /** Types configured in the admin settings; the required choice is sent as `type`. */
  settlementTypes: string[];
  /** Called once the editor is finished: the settlement was saved or editing stopped. */
  onDone?: () => void;
  /** Drops the section chrome for dialog embedding; the province page keeps the default. */
  compact?: boolean;
};

type Coordinates = {
  latitude: number;
  longitude: number;
};

/** The server stores one "latitude, longitude" string, so the field parses it the same way. */
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

export default function SettlementEditor({
  guberniaId,
  provinces,
  settlementTypes,
  onDone,
  compact,
}: SettlementEditorProps) {
  const router = useRouter();
  const headingId = useId();
  const nameId = useId();
  const slugId = useId();
  const provinceId = useId();
  const districtId = useId();
  const typeId = useId();
  const coordinatesId = useId();

  const [name, setName] = useState("");
  const [selectedProvinceId, setSelectedProvinceId] = useState(guberniaId);
  const [selectedDistrictId, setSelectedDistrictId] = useState("");
  const [typeValue, setTypeValue] = useState("");
  const [coordinates, setCoordinates] = useState("");
  const [slug, setSlug] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const nameRef = useRef<HTMLInputElement>(null);
  const slugRef = useRef<HTMLInputElement>(null);
  const provinceRef = useRef<HTMLSelectElement>(null);
  const districtRef = useRef<HTMLSelectElement>(null);
  const typeRef = useRef<HTMLSelectElement>(null);
  const coordinatesRef = useRef<HTMLInputElement>(null);

  const { districts, loading: districtsLoading, error: districtsError } =
    useDistrictOptions(selectedProvinceId);

  function resetDraft() {
    setName("");
    setSlug("");
    setSelectedProvinceId(guberniaId);
    setSelectedDistrictId("");
    setTypeValue("");
    setCoordinates("");
  }

  function cancel() {
    if (saving) return;
    setError("");
    setStatus("");
    if (onDone) {
      // The panel owns the editor; it closes it on this signal.
      onDone();
      return;
    }
    resetDraft();
    nameRef.current?.focus();
  }

  async function save() {
    if (saving) return;
    const nextName = name.trim();
    if (!nextName) {
      setStatus("");
      setError("Введите название населённого пункта.");
      nameRef.current?.focus();
      return;
    }
    const nextSlug = slug.trim();
    if (!nextSlug) {
      setStatus("");
      setError("Укажите адрес страницы населённого пункта.");
      slugRef.current?.focus();
      return;
    }
    if (nextSlug.length > SETTLEMENT_URL_SLUG_MAX_LENGTH) {
      setStatus("");
      setError(`Адрес страницы должен быть не длиннее ${SETTLEMENT_URL_SLUG_MAX_LENGTH} символов.`);
      slugRef.current?.focus();
      return;
    }
    if (!isValidSettlementSlug(nextSlug)) {
      setStatus("");
      setError("Адрес должен содержать только строчные латинские буквы, цифры и одиночные дефисы.");
      slugRef.current?.focus();
      return;
    }
    if (!selectedProvinceId) {
      setStatus("");
      setError("Выберите губернию.");
      provinceRef.current?.focus();
      return;
    }
    if (districtsLoading) {
      setStatus("");
      setError("Дождитесь загрузки списка уездов.");
      return;
    }
    if (districtsError) {
      setStatus("");
      setError(districtsError);
      return;
    }
    if (!selectedDistrictId) {
      setStatus("");
      setError("Выберите уезд.");
      districtRef.current?.focus();
      return;
    }
    if (!typeValue) {
      setStatus("");
      setError("Выберите тип населённого пункта.");
      typeRef.current?.focus();
      return;
    }
    const parsed = parseCoordinates(coordinates);
    if (!parsed) {
      setStatus("");
      setError("Укажите координаты в формате «широта, долгота», например 56.1234, 40.5678.");
      coordinatesRef.current?.focus();
      return;
    }

    setSaving(true);
    setError("");
    setStatus("");
    try {
      const response = await fetch(
        `/api/gubernias/${encodeURIComponent(selectedProvinceId)}/settlements`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          credentials: "same-origin",
          body: JSON.stringify({
            name: nextName,
            uyezdId: selectedDistrictId,
            type: typeValue,
            url: settlementUrlForSlug(nextSlug),
            coordinates: `${parsed.latitude}, ${parsed.longitude}`,
          }),
        },
      );
      if (!response.ok) {
        setError(await errorMessage(response, "Не удалось сохранить населённый пункт. Попробуйте ещё раз."));
        return;
      }

      router.refresh();
      if (onDone) {
        onDone();
        return;
      }
      // The settlement is stored already, so the form goes back to a blank draft.
      resetDraft();
      setStatus("Населённый пункт добавлен.");
      nameRef.current?.focus();
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
    event.preventDefault();
    cancel();
  }

  return (
    <section
      className={compact ? `${styles.panel} ${styles.compact}` : styles.panel}
      aria-labelledby={headingId}
    >
      <h2 id={headingId} className={styles.heading}>
        Новый населённый пункт
      </h2>
      <form className={styles.form} onSubmit={handleSubmit} onKeyDown={handleFormKeyDown} noValidate>
        <label htmlFor={nameId}>Название населённого пункта</label>
        <input
          ref={nameRef}
          id={nameId}
          name="name"
          className={styles.input}
          value={name}
          maxLength={NAME_MAX_LENGTH}
          autoComplete="off"
          spellCheck={false}
          required
          onChange={(event) => {
            setName(event.target.value);
            setStatus("");
          }}
        />

        <label htmlFor={slugId}>Адрес страницы</label>
        <div className={styles.addressField}>
          <span className={styles.addressPrefix}>{SETTLEMENT_URL_PREFIX}</span>
          <input
            ref={slugRef}
            id={slugId}
            name="slug"
            className={styles.addressInput}
            type="text"
            value={slug}
            maxLength={SETTLEMENT_URL_SLUG_MAX_LENGTH}
            placeholder="ivanovo"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            required
            onChange={(event) => {
              setSlug(event.target.value);
              setStatus("");
            }}
          />
        </div>
        <p className={styles.hint}>
          Локальный адрес латиницей: строчные буквы, цифры и дефисы.
        </p>

        <div className={styles.fields}>
          <div className={styles.field}>
            <label htmlFor={provinceId}>Губерния</label>
            <select
              ref={provinceRef}
              id={provinceId}
              name="guberniaId"
              className={styles.select}
              value={selectedProvinceId}
              onChange={(event) => {
                setSelectedProvinceId(event.target.value);
                // The district list belongs to the province, so the choice falls back.
                setSelectedDistrictId("");
                setStatus("");
              }}
            >
              <option value="">— Не выбрана —</option>
              {provinces.map((province) => (
                <option key={province.id} value={province.id}>
                  {province.name}
                </option>
              ))}
            </select>
          </div>

          <div className={styles.field}>
            <label htmlFor={districtId}>Уезд</label>
            <select
              ref={districtRef}
              id={districtId}
              name="uyezdId"
              className={styles.select}
              value={selectedDistrictId}
              disabled={!selectedProvinceId || districtsLoading}
              onChange={(event) => {
                setSelectedDistrictId(event.target.value);
                setStatus("");
              }}
            >
              <option value="">{districtsLoading ? "Загружаем…" : "— Не указан —"}</option>
              {districts.map((district) => (
                <option key={district.id} value={district.id}>
                  {district.name}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div className={styles.field}>
          <label htmlFor={typeId}>Тип населённого пункта</label>
          <select
            ref={typeRef}
            id={typeId}
            name="type"
            className={styles.select}
            value={typeValue}
            required
            onChange={(event) => {
              setTypeValue(event.target.value);
              setStatus("");
            }}
          >
            <option value="">— Не выбран —</option>
            {settlementTypes.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </div>

        <label htmlFor={coordinatesId}>Координаты (широта, долгота)</label>
        <input
          ref={coordinatesRef}
          id={coordinatesId}
          name="coordinates"
          className={styles.input}
          type="text"
          inputMode="decimal"
          value={coordinates}
          placeholder="Например, 56.1234, 40.5678"
          autoComplete="off"
          spellCheck={false}
          required
          onChange={(event) => {
            setCoordinates(event.target.value);
            setStatus("");
          }}
        />
        <p className={styles.hint}>
          Точка должна находиться в границах выбранного уезда — по ней населённый пункт появится на карте.
        </p>

        {districtsError ? <p className={styles.hint}>{districtsError}</p> : null}
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
          <button className={styles.primaryButton} type="submit" disabled={saving}>
            {saving ? "Сохраняем…" : "Сохранить"}
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
    const body = (await response.json()) as { error?: unknown };
    if (typeof body.error === "string" && body.error.trim()) return body.error;
  } catch {
    /* Non-JSON error body: fall back to the generic message. */
  }
  return fallback;
}
