import { randomUUID } from "node:crypto";
import { mkdir, open, rename, unlink, type FileHandle } from "node:fs/promises";
import path from "node:path";
import { POST_CATEGORIES } from "@/lib/post-categories";

export type SiteSettings = {
  categories: string[];
  settlementTypes: string[];
};

export type SiteSettingKind = keyof SiteSettings;

export class SiteSettingsValidationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SiteSettingsValidationError";
  }
}

type StoredSiteSettings = SiteSettings & {
  version: 1;
};

type SiteSettingsRuntime = {
  queue: Promise<void>;
};

const DEFAULT_SETTINGS_PATH = path.join(process.cwd(), "data", "site-settings.json");
const DEFAULT_SETTLEMENT_TYPES = ["Город", "Село", "Деревня"] as const;
const STORED_KEYS: Record<string, true> = {
  version: true,
  categories: true,
  settlementTypes: true,
};
const MAX_NAME_CHARACTERS = 80;
const MAX_ITEMS_PER_LIST = 100;
const MAX_STORED_BYTES = 65_536;
const RUNTIME_KEY = Symbol.for("dimasik.site-settings.runtime");
const siteSettingsGlobal = globalThis as typeof globalThis & {
  [key: symbol]: SiteSettingsRuntime | undefined;
};
const runtime: SiteSettingsRuntime = siteSettingsGlobal[RUNTIME_KEY] ?? {
  queue: Promise.resolve(),
};
siteSettingsGlobal[RUNTIME_KEY] = runtime;

function serialized<T>(operation: () => Promise<T>): Promise<T> {
  const result = runtime.queue.then(operation, operation);
  runtime.queue = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

function settingsPath() {
  const configuredPath = process.env.SITE_SETTINGS_PATH;
  return configuredPath ? path.resolve(configuredPath) : DEFAULT_SETTINGS_PATH;
}

function normalizeName(value: unknown): string {
  if (typeof value !== "string") {
    throw new SiteSettingsValidationError("Введите название текстом.");
  }

  const name = value.trim().replace(/\s+/gu, " ");
  if (!name) {
    throw new SiteSettingsValidationError("Введите название.");
  }
  if (name.length > MAX_NAME_CHARACTERS) {
    throw new SiteSettingsValidationError(
      `Название не должно превышать ${MAX_NAME_CHARACTERS} символов.`,
    );
  }
  return name;
}

function parseList(value: unknown, label: string): string[] {
  if (!Array.isArray(value) || value.length > MAX_ITEMS_PER_LIST) {
    throw new Error(`Site settings ${label} list has an unsupported format.`);
  }

  const result: string[] = [];
  const seen = new Set<string>();
  for (const valueItem of value) {
    let item: string;
    try {
      item = normalizeName(valueItem);
    } catch (error) {
      throw new Error(`Site settings ${label} list contains an invalid value.`, {
        cause: error,
      });
    }
    if (seen.has(item)) {
      throw new Error(`Site settings ${label} list contains a duplicate value.`);
    }
    seen.add(item);
    result.push(item);
  }
  return result;
}

function parseStoredSettings(source: string): SiteSettings {
  let parsed: unknown;
  try {
    parsed = JSON.parse(source);
  } catch (error) {
    throw new Error("Site settings storage contains invalid JSON.", { cause: error });
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("Site settings storage is not an object.");
  }
  const candidate = parsed as Record<string, unknown>;
  if (
    candidate.version !== 1 ||
    Object.keys(candidate).length !== 3 ||
    !Object.keys(candidate).every((key) => STORED_KEYS[key] === true)
  ) {
    throw new Error("Site settings storage has an unsupported format.");
  }

  return {
    categories: parseList(candidate.categories, "categories"),
    settlementTypes: parseList(candidate.settlementTypes, "settlement types"),
  };
}

async function readBoundedFile(filePath: string): Promise<string> {
  const handle = await open(filePath, "r");
  try {
    const { size } = await handle.stat();
    if (!Number.isSafeInteger(size) || size > MAX_STORED_BYTES) {
      throw new Error("Site settings storage is too large.");
    }

    const buffer = Buffer.allocUnsafe(size + 1);
    let offset = 0;
    while (offset < buffer.length) {
      const { bytesRead } = await handle.read(buffer, offset, buffer.length - offset, offset);
      if (bytesRead === 0) break;
      offset += bytesRead;
    }
    if (offset > MAX_STORED_BYTES) {
      throw new Error("Site settings storage is too large.");
    }

    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, offset));
    } catch (error) {
      throw new Error("Site settings storage is not valid UTF-8.", { cause: error });
    }
  } finally {
    await handle.close();
  }
}

async function readSettings(filePath: string): Promise<SiteSettings> {
  try {
    return parseStoredSettings(await readBoundedFile(filePath));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    // The store file is created by the first successful addition, so a
    // read-only deployment still serves the bundled defaults.
    return {
      categories: [...POST_CATEGORIES],
      settlementTypes: [...DEFAULT_SETTLEMENT_TYPES],
    };
  }
}

async function writeSettings(filePath: string, settings: SiteSettings) {
  const stored: StoredSiteSettings = { version: 1, ...settings };
  const source = `${JSON.stringify(stored)}\n`;
  if (Buffer.byteLength(source, "utf8") > MAX_STORED_BYTES) {
    throw new SiteSettingsValidationError("Список настроек достиг предельного размера.");
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

export async function getSiteSettings(): Promise<SiteSettings> {
  return serialized(() => readSettings(settingsPath()));
}

export async function addSiteSetting(
  kind: SiteSettingKind,
  value: unknown,
): Promise<SiteSettings> {
  if (kind !== "categories" && kind !== "settlementTypes") {
    throw new SiteSettingsValidationError("Неизвестный список настроек.");
  }

  const name = normalizeName(value);
  return serialized(async () => {
    const filePath = settingsPath();
    const settings = await readSettings(filePath);
    const values = settings[kind];
    if (values.includes(name)) {
      throw new SiteSettingsValidationError("Такое название уже есть в списке.");
    }
    if (values.length >= MAX_ITEMS_PER_LIST) {
      throw new SiteSettingsValidationError("Список настроек достиг предельного размера.");
    }

    const nextSettings: SiteSettings = {
      categories: kind === "categories" ? [...settings.categories, name] : settings.categories,
      settlementTypes:
        kind === "settlementTypes" ? [...settings.settlementTypes, name] : settings.settlementTypes,
    };
    await writeSettings(filePath, nextSettings);
    return nextSettings;
  });
}

export async function renameSiteSetting(
  kind: SiteSettingKind,
  originalName: unknown,
  value: unknown,
): Promise<SiteSettings> {
  if (kind !== "categories" && kind !== "settlementTypes") {
    throw new SiteSettingsValidationError("Неизвестный список настроек.");
  }
  if (typeof originalName !== "string") {
    throw new SiteSettingsValidationError("Исходное название не найдено.");
  }

  const name = normalizeName(value);
  return serialized(async () => {
    const filePath = settingsPath();
    const settings = await readSettings(filePath);
    const values = settings[kind];
    const index = values.indexOf(originalName);
    if (index === -1) {
      throw new SiteSettingsValidationError("Исходное название не найдено.");
    }
    if (name !== originalName && values.includes(name)) {
      throw new SiteSettingsValidationError("Такое название уже есть в списке.");
    }

    const nextValues = [...values];
    nextValues[index] = name;
    const nextSettings: SiteSettings = {
      categories: kind === "categories" ? nextValues : settings.categories,
      settlementTypes: kind === "settlementTypes" ? nextValues : settings.settlementTypes,
    };
    await writeSettings(filePath, nextSettings);
    return nextSettings;
  });
}

export async function deleteSiteSetting(
  kind: SiteSettingKind,
  name: unknown,
): Promise<SiteSettings> {
  if (kind !== "categories" && kind !== "settlementTypes") {
    throw new SiteSettingsValidationError("Неизвестный список настроек.");
  }
  if (typeof name !== "string") {
    throw new SiteSettingsValidationError("Название не найдено.");
  }

  return serialized(async () => {
    const filePath = settingsPath();
    const settings = await readSettings(filePath);
    const values = settings[kind];
    const index = values.indexOf(name);
    if (index === -1) {
      throw new SiteSettingsValidationError("Название не найдено.");
    }

    const nextValues = [...values.slice(0, index), ...values.slice(index + 1)];
    const nextSettings: SiteSettings = {
      categories: kind === "categories" ? nextValues : settings.categories,
      settlementTypes: kind === "settlementTypes" ? nextValues : settings.settlementTypes,
    };
    await writeSettings(filePath, nextSettings);
    return nextSettings;
  });
}
