"use client";

import {
  useEffect,
  useId,
  useImperativeHandle,
  useRef,
  useState,
  type ChangeEvent,
  type Ref,
} from "react";
import type { PostImage } from "@/lib/gubernia-publications";
import styles from "./PostImageUploader.module.css";

const UPLOAD_URL = "/api/post-images";
const MAX_IMAGES = 10;
const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
const MAX_IMAGE_MEGABYTES = MAX_IMAGE_BYTES / (1024 * 1024);
const FILE_INPUT_ACCEPT = "image/jpeg,image/png,image/webp";
/** Static lookup tables: accepted mime types, plus an extension fallback. */
const ACCEPTED_MIME_TYPES: Record<string, true> = {
  "image/jpeg": true,
  "image/png": true,
  "image/webp": true,
};
const ACCEPTED_EXTENSIONS: Record<string, true> = {
  ".jpg": true,
  ".jpeg": true,
  ".png": true,
  ".webp": true,
};

type UploadStatus = "uploading" | "uploaded" | "error";

type UploadItem = {
  key: string;
  name: string;
  /** Local object URL for new uploads, or the durable thumbnail URL for saved images. */
  previewUrl: string;
  source: "persisted" | "pending";
  status: UploadStatus;
  /** Upload percentage 0-100; only meaningful while the upload is in flight. */
  progress: number;
  imageId: string | null;
  error: string;
};

/** Imperative escape hatch for the form that owns save/cancel semantics. */
export type PostImageUploaderHandle = {
  /** Final image ids in display order: retained saved images followed by new uploads. */
  getImageIds: () => string[];
  /** True while an upload is in flight or a failed image is still attached. */
  isBlocked: () => boolean;
  /**
   * Marks a save request as in flight. While set, unmount cleanup leaves new
   * pending uploads to the server TTL sweep because the request may claim them.
   */
  setSaveInFlight: (inFlight: boolean) => void;
  /** Hands new pending ids to a successfully stored post. */
  claimAll: () => void;
  /** Deletes new uploaded-but-unclaimed images; saved images are never touched. */
  cleanupPending: () => void;
  /** Restores the initial saved-image selection and drops local pending thumbnails. */
  reset: () => void;
};

type PostImageUploaderProps = {
  /** Images already attached to an edited post, in display order. */
  initialImages?: PostImage[];
  /** True while the post itself is being saved. */
  disabled?: boolean;
  /** Reports whether the form must stay unsubmittable: uploading or failed items. */
  onBlockedChange?: (blocked: boolean) => void;
  ref?: Ref<PostImageUploaderHandle>;
};

/**
 * Post image picker: retains saved images without owning their deletion, uploads
 * new files one at a time, and deletes only pending uploads when discarded.
 * The server owns real validation (decoding, size, pixel count), so the checks
 * here are only early feedback.
 */
export default function PostImageUploader({
  initialImages = [],
  disabled = false,
  onBlockedChange,
  ref,
}: PostImageUploaderProps) {
  const inputId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [items, setItems] = useState<UploadItem[]>(() => seedPersistedImages(initialImages));
  const [listError, setListError] = useState("");
  // The initial snapshot deliberately never follows prop identity changes:
  // rerenders must not undo staged removals. A remount seeds the post afresh.
  const initialItemsRef = useRef(items);
  // The ref is the mutable source of truth: upload callbacks finish long after
  // the render that started them and must not read a stale state snapshot.
  const itemsRef = useRef(items);
  /** Keys still wanted by the form; a missing key means a pending item was dropped. */
  const activeKeysRef = useRef<Set<string>>(new Set());
  const liveUrlsRef = useRef<Set<string>>(new Set());
  /** New uploaded ids that this editor owns and may delete. */
  const pendingIdsRef = useRef<Set<string>>(new Set());
  /** New ids handed to a stored post; deleting those is never allowed. */
  const claimedIdsRef = useRef<Set<string>>(new Set());
  /** Prevents unmount cleanup from racing a save request that may claim new ids. */
  const saveInFlightRef = useRef(false);
  const uploadQueueRef = useRef<Promise<void>>(Promise.resolve());
  const nextKeyRef = useRef(0);

  function commit(next: UploadItem[]) {
    itemsRef.current = next;
    setItems(next);
  }

  function updateItem(key: string, patch: Partial<UploadItem>) {
    commit(itemsRef.current.map((item) => (item.key === key ? { ...item, ...patch } : item)));
  }

  function revokePreview(url: string) {
    // The set guards against revoking a URL twice, including on remounts.
    if (liveUrlsRef.current.delete(url)) URL.revokeObjectURL(url);
  }

  /** Deletes an uploaded-but-unclaimed image; claimed ids are left alone. */
  function deletePending(imageId: string) {
    if (claimedIdsRef.current.has(imageId)) return;
    // Dropping the id from the pending set first also makes the call idempotent.
    if (!pendingIdsRef.current.delete(imageId)) return;
    void fetch(`${UPLOAD_URL}/${encodeURIComponent(imageId)}`, {
      method: "DELETE",
      credentials: "same-origin",
      // Lets the request finish when the editor unmounts during navigation.
      keepalive: true,
    }).catch(() => {
      /* Best effort: abandoned pending uploads are swept by the server by age. */
    });
  }

  function cleanupPending() {
    for (const imageId of [...pendingIdsRef.current]) deletePending(imageId);
  }

  function setSaveInFlight(inFlight: boolean) {
    saveInFlightRef.current = inFlight;
  }

  function reset() {
    for (const url of [...liveUrlsRef.current]) revokePreview(url);
    activeKeysRef.current.clear();
    commit(initialItemsRef.current);
    setListError("");
    if (inputRef.current) inputRef.current.value = "";
  }

  function claimAll() {
    for (const item of itemsRef.current) {
      if (!item.imageId) continue;
      if (pendingIdsRef.current.delete(item.imageId)) claimedIdsRef.current.add(item.imageId);
    }
  }

  function removeItem(key: string) {
    const item = itemsRef.current.find((entry) => entry.key === key);
    if (!item) return;
    activeKeysRef.current.delete(key);
    commit(itemsRef.current.filter((entry) => entry.key !== key));
    revokePreview(item.previewUrl);
    if (item.imageId) deletePending(item.imageId);
  }

  async function runUpload(key: string, file: File) {
    // The item was dropped before its turn came: never even start the request.
    // Requests already in flight are not aborted on purpose (see below).
    if (!activeKeysRef.current.has(key)) return;
    let image: PostImage;
    try {
      image = await uploadPostImage(file, (percent) => {
        if (!activeKeysRef.current.has(key)) return;
        const current = itemsRef.current.find((item) => item.key === key);
        if (!current || current.progress === percent) return;
        updateItem(key, { progress: percent });
      });
    } catch (thrown) {
      if (!activeKeysRef.current.has(key)) return;
      const message =
        thrown instanceof Error && thrown.message
          ? thrown.message
          : "Не удалось загрузить изображение.";
      updateItem(key, { status: "error", error: message });
      return;
    }
    if (!activeKeysRef.current.has(key)) {
      // The item was removed or the draft was cancelled while the file was in
      // flight. Aborting could leak an id the page never sees, so the request
      // is left to finish and the pending object it returned is deleted here.
      pendingIdsRef.current.add(image.id);
      deletePending(image.id);
      return;
    }
    pendingIdsRef.current.add(image.id);
    updateItem(key, { status: "uploaded", progress: 100, imageId: image.id, error: "" });
  }

  function startUpload(file: File) {
    const key = `image-${nextKeyRef.current++}`;
    const previewUrl = URL.createObjectURL(file);
    liveUrlsRef.current.add(previewUrl);
    activeKeysRef.current.add(key);
    commit([
      ...itemsRef.current,
      {
        key,
        name: file.name,
        previewUrl,
        source: "pending",
        status: "uploading",
        progress: 0,
        imageId: null,
        error: "",
      },
    ]);
    // One file at a time, so the ids land in the order the author picked them.
    uploadQueueRef.current = uploadQueueRef.current
      .then(() => runUpload(key, file))
      .catch(() => {});
  }

  function handleFilesSelected(event: ChangeEvent<HTMLInputElement>) {
    const input = event.currentTarget;
    const files = Array.from(input.files ?? []);
    // Allows picking the same file again after removing it.
    input.value = "";
    if (files.length === 0) return;
    const problems: string[] = [];
    let remaining = MAX_IMAGES - itemsRef.current.length;
    for (const file of files) {
      if (remaining <= 0) {
        problems.push(`«${file.name}» — можно приложить не больше ${MAX_IMAGES} изображений.`);
        continue;
      }
      const problem = imageFileProblem(file);
      if (problem) {
        problems.push(`«${file.name}» — ${problem}`);
        continue;
      }
      startUpload(file);
      remaining -= 1;
    }
    setListError(
      problems.length > 0
        ? `${problems.join(" ")} Эти файлы не будут приложены к сообщению.`
        : "",
    );
  }

  useEffect(() => {
    return () => {
      // A create request may already have claimed these ids even if its response
      // never reaches this component. Leave that uncertain outcome to the TTL sweep.
      if (!saveInFlightRef.current) cleanupPending();
      for (const url of [...liveUrlsRef.current]) revokePreview(url);
      activeKeysRef.current.clear();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- refs are stable; cleanup stays idempotent.
  }, []);

  const uploading = items.filter((item) => item.status === "uploading").length;
  const failed = items.filter((item) => item.status === "error").length;
  const ready = items.filter((item) => item.status === "uploaded" && item.source === "pending").length;
  const blocked = uploading > 0 || failed > 0;
  const removedPersisted =
    initialItemsRef.current.filter((item) => item.source === "persisted").length -
    items.filter((item) => item.source === "persisted").length;

  useEffect(() => {
    onBlockedChange?.(blocked);
  }, [blocked, onBlockedChange]);

  useImperativeHandle(ref, () => ({
    getImageIds: () =>
      itemsRef.current
        .filter((item): item is UploadItem & { imageId: string } => item.imageId !== null)
        .map((item) => item.imageId),
    isBlocked: () =>
      itemsRef.current.some((item) => item.status === "uploading" || item.status === "error"),
    setSaveInFlight,
    claimAll,
    cleanupPending,
    reset,
  }));

  // Only meaningful transitions are announced; per-percent changes are visual.
  const summary =
    uploading > 0
      ? `Идёт загрузка изображений: осталось ${uploading}.`
      : failed > 0
        ? `Не удалось загрузить изображения: ${failed}. Удалите их или добавьте другие файлы.`
        : ready > 0
          ? `Изображения загружены: ${ready}.`
          : "";

  return (
    <div className={styles.uploader}>
      <label className={styles.uploadLabel} htmlFor={inputId}>
        Изображения
      </label>
      <input
        ref={inputRef}
        id={inputId}
        className={styles.fileInput}
        type="file"
        accept={FILE_INPUT_ACCEPT}
        multiple
        disabled={disabled}
        onChange={handleFilesSelected}
      />
      <p className={styles.hint}>
        {`Не больше ${MAX_IMAGES} изображений всего, каждый новый файл до ${MAX_IMAGE_MEGABYTES} МБ: JPEG, PNG или WebP.`}
      </p>
      {listError ? (
        <p className={styles.error} role="alert">
          {listError}
        </p>
      ) : null}
      {items.length > 0 ? (
        <ul className={styles.list}>
          {items.map((item) => (
            <li key={item.key} className={styles.item}>
              {/* eslint-disable-next-line @next/next/no-img-element -- Local object URL preview cannot use next/image. */}
              <img className={styles.preview} src={item.previewUrl} alt="" />
              <div className={styles.itemBody}>
                <span className={styles.itemName}>{item.name}</span>
                {item.status === "uploading" ? (
                  <span className={styles.itemState}>
                    {`Загрузка… ${item.progress}%`}
                    <span className={styles.progressTrack} aria-hidden="true">
                      <span className={styles.progressFill} style={{ width: `${item.progress}%` }} />
                    </span>
                  </span>
                ) : item.status === "error" ? (
                  <span className={styles.itemError}>{item.error}</span>
                ) : (
                  <span className={styles.itemState}>
                    {item.source === "persisted" ? "Сохранено" : "Готово к отправке"}
                  </span>
                )}
              </div>
              <button
                className={styles.removeButton}
                type="button"
                disabled={disabled}
                aria-label={`Удалить изображение «${item.name}»`}
                onClick={() => removeItem(item.key)}
              >
                Удалить
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {removedPersisted > 0 ? (
        <p className={styles.hint}>
          Удаление сохранённых изображений применится только после сохранения сообщения.
        </p>
      ) : null}
      {summary ? (
        <p className={styles.status} role="status">
          {summary}
          {blocked ? " Пока изображения загружаются или есть ошибки, сохранить сообщение нельзя." : ""}
        </p>
      ) : null}
    </div>
  );
}

function seedPersistedImages(images: PostImage[]): UploadItem[] {
  return images.map((image, index) => ({
    key: `persisted-${image.id}`,
    name: `Сохранённое изображение ${index + 1}`,
    previewUrl: image.thumbnailUrl,
    source: "persisted",
    status: "uploaded",
    progress: 100,
    imageId: image.id,
    error: "",
  }));
}

/** Early client-side filter; the backend still decodes and validates for real. */
function imageFileProblem(file: File): string | null {
  if (!isAcceptedImage(file)) return "поддерживаются только JPEG, PNG и WebP.";
  if (file.size > MAX_IMAGE_BYTES) return `размер больше ${MAX_IMAGE_MEGABYTES} МБ.`;
  return null;
}

/** Browsers may report an empty type, so the extension is the fallback. */
function isAcceptedImage(file: File): boolean {
  if (ACCEPTED_MIME_TYPES[file.type]) return true;
  const name = file.name.toLowerCase();
  return Object.keys(ACCEPTED_EXTENSIONS).some((extension) => name.endsWith(extension));
}

function uploadPostImage(file: File, onProgress: (percent: number) => void): Promise<PostImage> {
  return new Promise<PostImage>((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("POST", UPLOAD_URL);
    // fetch cannot report upload progress, so the picker uses XHR here.
    request.responseType = "json";
    request.withCredentials = true;

    request.upload.addEventListener("progress", (event) => {
      if (!event.lengthComputable || event.total === 0) return;
      // 100% is only reached once the server has answered.
      onProgress(Math.min(99, Math.round((event.loaded / event.total) * 100)));
    });

    request.addEventListener("load", () => {
      const payload: unknown = request.response;
      if (request.status >= 200 && request.status < 300) {
        const image = readUploadedImage(payload);
        if (image) {
          onProgress(100);
          resolve(image);
          return;
        }
        reject(new Error("Сервер вернул неожиданный ответ при загрузке изображения."));
        return;
      }
      reject(new Error(uploadErrorMessage(payload, request.status)));
    });
    request.addEventListener("error", () => {
      reject(new Error("Не удалось связаться с сервером."));
    });
    request.addEventListener("abort", () => {
      // The picker never aborts on purpose; a browser abort must not leave the queue stuck.
      reject(new Error("Загрузка изображения прервана. Попробуйте ещё раз."));
    });

    const form = new FormData();
    form.append("file", file);
    request.send(form);
  });
}

/**
 * Keeps the id even from a partially unusual body: losing it would orphan the
 * pending upload the server may have just created.
 */
function readUploadedImage(payload: unknown): PostImage | null {
  if (!payload || typeof payload !== "object") return null;
  if (!("id" in payload) || typeof payload.id !== "string" || !payload.id.trim()) return null;
  const id = payload.id;
  const width =
    "width" in payload && typeof payload.width === "number" && Number.isFinite(payload.width)
      ? payload.width
      : 0;
  const height =
    "height" in payload && typeof payload.height === "number" && Number.isFinite(payload.height)
      ? payload.height
      : 0;
  const originalUrl =
    "originalUrl" in payload && typeof payload.originalUrl === "string"
      ? payload.originalUrl
      : `${UPLOAD_URL}/${encodeURIComponent(id)}/original`;
  const thumbnailUrl =
    "thumbnailUrl" in payload && typeof payload.thumbnailUrl === "string"
      ? payload.thumbnailUrl
      : `${UPLOAD_URL}/${encodeURIComponent(id)}/thumbnail`;
  return { id, width, height, originalUrl, thumbnailUrl };
}

function uploadErrorMessage(payload: unknown, status: number): string {
  if (payload && typeof payload === "object" && "error" in payload) {
    const message = payload.error;
    if (typeof message === "string" && message.trim()) return message;
  }
  if (status === 503) return "Загрузка изображений недоступна: хранилище не настроено.";
  if (status === 413) return "Файл слишком большой.";
  if (status === 415) return "Файл не является изображением JPEG, PNG или WebP.";
  if (status === 401 || status === 403) return "Нет прав для загрузки изображений.";
  return "Не удалось загрузить изображение. Попробуйте ещё раз.";
}
