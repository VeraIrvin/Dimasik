import { randomUUID } from "node:crypto";
import { mkdir, open, rename, unlink, type FileHandle } from "node:fs/promises";
import path from "node:path";
import {
  MAX_POST_DOCUMENT_JSON_CHARACTERS,
  normalizePostDocument,
  type PostDocument,
} from "@/lib/post-content";

const DEFAULT_CONTENT_PATH = path.join(process.cwd(), "data", "about-content.json");
// JSON.stringify bounds post documents by UTF-16 code units, each occupying at
// most three UTF-8 bytes; the extra KB covers the version envelope and newline.
const MAX_STORED_BYTES = MAX_POST_DOCUMENT_JSON_CHARACTERS * 3 + 1_024;
const STORED_KEYS: Record<string, true> = { version: true, body: true };

const DEFAULT_PARAGRAPHS = [
  "Это прототип интерфейса для изучения губерний Российской империи и будущей работы с архивными документами.",
  "Карта показывает историческую реконструкцию границ 76 губерний по состоянию на 1897 год. Изначально десять опубликованных губерний выделены бежевым и представлены в списке поверх карты. Наведение на губернию в списке или на карте подсвечивает обе её формы представления; клик открывает страницу губернии. Остальные губернии показаны бледным контекстом, не подсвечиваются и не открываются по клику. Другие территории того же среза образуют нейтральный фон без границ и подписей. Современная географическая подложка не используется.",
  "Вход администратора расположен на главной странице. Администратор может добавить губернию из исторического слоя в список доступных, изменить адрес и описание её страницы или удалить страницу. Полигон удалённой губернии остаётся нейтральным контекстом карты.",
  "Сообщения на страницах губерний доступны всем посетителям. Боковой список заголовков помогает перейти к нужному сообщению; администратор может публиковать, редактировать и удалять сообщения с форматированным текстом.",
  "На странице каждой опубликованной губернии под её названием есть карта границ уездов и округов. Список слева и карта подсвечивают одну и ту же единицу; нажатие приближает к ней, но отдельные страницы уездов пока не создаются.",
] as const;

type StoredAboutContent = {
  version: 1;
  body: PostDocument;
};

type AboutContentRuntime = {
  queue: Promise<void>;
};

const RUNTIME_KEY = Symbol.for("dimasik.about-content.runtime");
const aboutContentGlobal = globalThis as typeof globalThis & {
  [key: symbol]: AboutContentRuntime | undefined;
};
const runtime: AboutContentRuntime = aboutContentGlobal[RUNTIME_KEY] ?? {
  queue: Promise.resolve(),
};
aboutContentGlobal[RUNTIME_KEY] = runtime;

function serialized<T>(operation: () => Promise<T>): Promise<T> {
  const result = runtime.queue.then(operation, operation);
  runtime.queue = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

function contentPath() {
  const configuredPath = process.env.ABOUT_CONTENT_PATH;
  return configuredPath ? path.resolve(configuredPath) : DEFAULT_CONTENT_PATH;
}

function parseStoredContent(source: string): PostDocument {
  let parsed: unknown;
  try {
    parsed = JSON.parse(source);
  } catch (error) {
    throw new Error("About content storage contains invalid JSON.", { cause: error });
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("About content storage is not an object.");
  }
  const candidate = parsed as Record<string, unknown>;
  if (
    candidate.version !== 1 ||
    Object.keys(candidate).length !== 2 ||
    !Object.keys(candidate).every((key) => STORED_KEYS[key] === true)
  ) {
    throw new Error("About content storage has an unsupported format.");
  }

  try {
    return normalizePostDocument(candidate.body);
  } catch (error) {
    throw new Error("About content storage contains an invalid document.", { cause: error });
  }
}

async function readBoundedFile(filePath: string): Promise<string> {
  const handle = await open(filePath, "r");
  try {
    const { size } = await handle.stat();
    if (!Number.isSafeInteger(size) || size > MAX_STORED_BYTES) {
      throw new Error("About content storage is too large.");
    }

    const buffer = Buffer.allocUnsafe(size + 1);
    let offset = 0;
    while (offset < buffer.length) {
      const { bytesRead } = await handle.read(buffer, offset, buffer.length - offset, offset);
      if (bytesRead === 0) break;
      offset += bytesRead;
    }
    if (offset > MAX_STORED_BYTES) {
      throw new Error("About content storage is too large.");
    }

    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, offset));
    } catch (error) {
      throw new Error("About content storage is not valid UTF-8.", { cause: error });
    }
  } finally {
    await handle.close();
  }
}

async function readContent(filePath: string): Promise<PostDocument> {
  try {
    return parseStoredContent(await readBoundedFile(filePath));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    // The store file is created by the first successful edit, so a read-only
    // deployment still serves the bundled About text.
    return {
      type: "doc",
      content: DEFAULT_PARAGRAPHS.map((text) => ({
        type: "paragraph",
        content: [{ type: "text", text }],
      })),
    };
  }
}

async function writeContent(filePath: string, body: PostDocument) {
  const stored: StoredAboutContent = { version: 1, body };
  const source = `${JSON.stringify(stored)}\n`;
  if (Buffer.byteLength(source, "utf8") > MAX_STORED_BYTES) {
    throw new Error("About content storage is too large.");
  }

  await mkdir(path.dirname(filePath), { recursive: true });
  const temporaryPath = `${filePath}.${process.pid}.${randomUUID()}.tmp`;
  let handle: FileHandle | undefined;
  try {
    handle = await open(temporaryPath, "wx", 0o600);
    await handle.writeFile(source, "utf8");
    await handle.sync();
    await handle.close();
    handle = undefined;
    await rename(temporaryPath, filePath);
  } catch (error) {
    await handle?.close().catch(() => undefined);
    await unlink(temporaryPath).catch(() => undefined);
    throw error;
  }
}

export async function getAboutContent(): Promise<PostDocument> {
  return serialized(() => readContent(contentPath()));
}

export async function saveAboutContent(value: unknown): Promise<PostDocument> {
  return serialized(async () => {
    const body = normalizePostDocument(value);
    await writeContent(contentPath(), body);
    return body;
  });
}
