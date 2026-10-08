import { randomUUID } from "node:crypto";
import { mkdir, open, readFile, rename, unlink, type FileHandle } from "node:fs/promises";
import path from "node:path";
import {
  PostContentValidationError,
  normalizePostDocument,
  type PostDocument,
} from "@/lib/post-content";
import type { PostCategory } from "@/lib/post-categories";
import { getSiteSettings } from "@/lib/site-settings";
import { getUyezd, isPointInUyezd } from "@/lib/uyezd-data";

const GEOMETRY_PATH = path.join(process.cwd(), "public", "data", "gubernias.geojson");
const DEFAULT_STATE_PATH = path.join(process.cwd(), "data", "gubernia-publications.json");
const SLUG_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const MAX_SLUG_LENGTH = 100;
const MAX_DESCRIPTION_LENGTH = 20_000;
const MAX_TITLE_LENGTH = 1000;
const MAX_ID_LENGTH = 100;
const MAX_SETTLEMENT_NAME_LENGTH = 200;
const MAX_YEAR_LENGTH = 100;
const MAX_ARCHIVE_REFERENCE_LENGTH = 300;
// Site settings reject names longer than 80 characters; stored values keep a
// margin above that cap so a settings change never makes old rows unreadable.
const MAX_STORED_SETTING_NAME_LENGTH = 200;
const SETTLEMENT_URL_PREFIX = "/naselennyy-punkt/";
const RUSSIAN_NAME_COLLATOR = new Intl.Collator("ru");
const STORED_POST_KEYS: Record<string, true> = {
  id: true,
  title: true,
  body: true,
  createdAt: true,
  updatedAt: true,
  uyezdId: true,
  settlementId: true,
  year: true,
  archiveReference: true,
  category: true,
};
const STORED_SETTLEMENT_KEYS: Record<string, true> = {
  id: true,
  name: true,
  guberniaId: true,
  uyezdId: true,
  latitude: true,
  longitude: true,
  createdAt: true,
  url: true,
  type: true,
};

export type GuberniaProperties = {
  id: string;
  name: string;
  published: boolean;
  slug?: string;
  labelCoordinates: [number, number];
  labelMinZoom: number;
};

export type GuberniaFeature = {
  type: "Feature";
  properties: GuberniaProperties;
  geometry: {
    type: string;
    coordinates: unknown;
  };
};

export type GuberniasCollection = {
  type: "FeatureCollection";
  features: GuberniaFeature[];
};

export type PublishedOption = {
  id: string;
  name: string;
  slug: string;
};

export type Settlement = {
  id: string;
  name: string;
  guberniaId: string;
  uyezdId: string;
  latitude: number;
  longitude: number;
  createdAt: string;
  url: string;
  /** Type chosen from the site settings; rows stored before types stay null. */
  type: string | null;
};

export { POST_CATEGORIES, type PostCategory } from "@/lib/post-categories";

export type GuberniaPost = {
  id: string;
  title: string;
  body: PostDocument;
  createdAt: string;
  updatedAt: string;
  uyezdId: string | null;
  settlementId: string | null;
  year: string;
  archiveReference: string;
  /** Posts stored before categories existed stay null until an edit picks one. */
  category: PostCategory | null;
};

export type PublishedGubernia = {
  id: string;
  name: string;
  slug: string;
  description: string;
  posts: GuberniaPost[];
  settlements: Settlement[];
};

/** A settlement page resolved by slug: the settlement plus its published province. */
export type PublishedSettlement = {
  settlement: Settlement;
  gubernia: PublishedGubernia;
};

export type PublishedGeoData = {
  provinces: PublishedOption[];
  settlements: Settlement[];
};

export type PublicationMetricsProvince = {
  id: string;
  name: string;
  slug: string;
  postsCount: number;
  settlementsCount: number;
};

/** Display-only content totals derived from the persisted publication state. */
export type PublicationMetrics = {
  totalHistoricalGubernias: number;
  publishedCount: number;
  totalPublishedPosts: number;
  totalPublishedSettlements: number;
  provinces: PublicationMetricsProvince[];
};

/** Placement and classification metadata accepted when a post is created or edited. */
export type PostMetadataInput = {
  uyezdId?: unknown;
  settlementId?: unknown;
  year?: unknown;
  archiveReference?: unknown;
  category?: unknown;
};

/** Edit metadata; `targetGuberniaId` may move the post to another province. */
export type PostUpdateInput = PostMetadataInput & {
  targetGuberniaId?: unknown;
};

type CanonicalProperties = Omit<GuberniaProperties, "published" | "slug"> & {
  published?: boolean;
  slug?: string;
};

type CanonicalFeature = Omit<GuberniaFeature, "properties"> & {
  properties: CanonicalProperties;
};

type CanonicalCollection = {
  type: "FeatureCollection";
  features: CanonicalFeature[];
};

type PublicationEntry = {
  published: boolean;
  slug: string | null;
  description: string;
  posts: GuberniaPost[];
  settlements: Settlement[];
};

type PublicationState = {
  version: 1;
  gubernias: Record<string, PublicationEntry>;
};

export class GuberniaPublicationError extends Error {
  constructor(
    message: string,
    readonly status: 400 | 404 | 409,
  ) {
    super(message);
    this.name = "GuberniaPublicationError";
  }
}

type PublicationsRuntime = {
  /** Queues every read and read-modify-write in this Node process. */
  queue: Promise<void>;
  /** Parsed canonical geometry shared by every bundled copy of this module. */
  collection?: Promise<CanonicalCollection>;
};

// Route and page bundles can duplicate this module, so the queue and geometry
// cache live behind a process-global symbol: one queue per Node instance.
// Separate processes writing the same file remain last-write-wins.
const RUNTIME_KEY = Symbol.for("dimasik.gubernia-publications.runtime");
const publicationsGlobal = globalThis as typeof globalThis & {
  [key: symbol]: PublicationsRuntime | undefined;
};
const runtime: PublicationsRuntime = publicationsGlobal[RUNTIME_KEY] ?? {
  queue: Promise.resolve(),
};
publicationsGlobal[RUNTIME_KEY] = runtime;

function serialized<T>(operation: () => Promise<T>): Promise<T> {
  const result = runtime.queue.then(operation, operation);
  runtime.queue = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

function statePath() {
  const configuredPath = process.env.GUBERNIAS_STATE_PATH;
  return configuredPath ? path.resolve(configuredPath) : DEFAULT_STATE_PATH;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function assertCanonicalCollection(value: unknown): asserts value is CanonicalCollection {
  if (typeof value !== "object" || value === null) {
    throw new Error("Canonical gubernia GeoJSON is not an object.");
  }

  const collection = value as { type?: unknown; features?: unknown };
  if (collection.type !== "FeatureCollection" || !Array.isArray(collection.features)) {
    throw new Error("Canonical gubernia GeoJSON is not a FeatureCollection.");
  }
  if (collection.features.length !== 76) {
    throw new Error(`Canonical gubernia GeoJSON must contain 76 features, found ${collection.features.length}.`);
  }

  const ids = new Set<string>();
  for (const feature of collection.features) {
    if (typeof feature !== "object" || feature === null) {
      throw new Error("Canonical gubernia GeoJSON contains an invalid feature.");
    }
    const candidate = feature as {
      type?: unknown;
      geometry?: unknown;
      properties?: Record<string, unknown>;
    };
    const properties = candidate.properties;
    if (
      candidate.type !== "Feature" ||
      typeof candidate.geometry !== "object" ||
      candidate.geometry === null ||
      typeof properties !== "object" ||
      properties === null ||
      typeof properties.id !== "string" ||
      properties.id.length === 0 ||
      typeof properties.name !== "string" ||
      properties.name.length === 0 ||
      !Array.isArray(properties.labelCoordinates) ||
      properties.labelCoordinates.length !== 2 ||
      !properties.labelCoordinates.every(isFiniteNumber) ||
      !isFiniteNumber(properties.labelMinZoom)
    ) {
      throw new Error("Canonical gubernia GeoJSON contains invalid feature metadata.");
    }
    if (ids.has(properties.id)) {
      throw new Error(`Canonical gubernia GeoJSON contains duplicate id ${properties.id}.`);
    }
    ids.add(properties.id);

    if (properties.published === true) {
      if (typeof properties.slug !== "string" || !isValidSlug(properties.slug)) {
        throw new Error(`Published canonical gubernia ${properties.id} has an invalid slug.`);
      }
    }
  }
}

function canonicalCollection(): Promise<CanonicalCollection> {
  const cached = runtime.collection;
  if (cached) return cached;

  const loaded: Promise<CanonicalCollection> = readFile(GEOMETRY_PATH, "utf8")
    .then((source) => {
      const parsed: unknown = JSON.parse(source);
      assertCanonicalCollection(parsed);
      return parsed;
    })
    .catch((error: unknown) => {
      // Never cache a failed read: the next request retries the file.
      runtime.collection = undefined;
      throw error;
    });
  runtime.collection = loaded;
  return loaded;
}

function isValidSlug(slug: string) {
  return slug.length <= MAX_SLUG_LENGTH && SLUG_PATTERN.test(slug);
}

function validateSlug(value: unknown) {
  if (typeof value !== "string" || !isValidSlug(value)) {
    throw new GuberniaPublicationError(
      "Slug должен содержать только строчные латинские буквы, цифры и одиночные дефисы.",
      400,
    );
  }
  return value;
}

function isValidSettlementUrl(url: string) {
  return (
    url.startsWith(SETTLEMENT_URL_PREFIX) &&
    isValidSlug(url.slice(SETTLEMENT_URL_PREFIX.length))
  );
}

function validateDescription(value: unknown) {
  if (typeof value !== "string" || value.length > MAX_DESCRIPTION_LENGTH) {
    throw new GuberniaPublicationError(
      `Описание должно быть строкой не длиннее ${MAX_DESCRIPTION_LENGTH} символов.`,
      400,
    );
  }
  return value;
}

function validatePostTitle(value: unknown) {
  if (typeof value !== "string") {
    throw new GuberniaPublicationError("Заголовок должен быть строкой.", 400);
  }
  const title = value.trim();
  if (title.length === 0) {
    throw new GuberniaPublicationError("Заголовок не должен быть пустым.", 400);
  }
  if (title.length > MAX_TITLE_LENGTH) {
    throw new GuberniaPublicationError(
      `Заголовок должен быть не длиннее ${MAX_TITLE_LENGTH} символов.`,
      400,
    );
  }
  return title;
}

function validatePostBody(value: unknown): PostDocument {
  try {
    return normalizePostDocument(value);
  } catch (error) {
    if (error instanceof PostContentValidationError) {
      throw new GuberniaPublicationError(error.message, 400);
    }
    throw error;
  }
}

function validatePostCategory(value: unknown, allowed: readonly string[]): PostCategory {
  if (typeof value !== "string" || !allowed.includes(value)) {
    throw new GuberniaPublicationError(
      `Категория должна быть одной из: ${allowed.map((category) => `«${category}»`).join(", ")}.`,
      400,
    );
  }
  return value;
}

function validateSettlementType(value: unknown, allowed: readonly string[]): string {
  if (typeof value !== "string" || !allowed.includes(value)) {
    throw new GuberniaPublicationError(
      `Тип населённого пункта должен быть одним из: ${allowed.map((type) => `«${type}»`).join(", ")}.`,
      400,
    );
  }
  return value;
}

function validateSettlementName(value: unknown) {
  if (typeof value !== "string") {
    throw new GuberniaPublicationError("Название населённого пункта должно быть строкой.", 400);
  }
  const name = value.trim();
  if (name.length === 0) {
    throw new GuberniaPublicationError("Название населённого пункта не должно быть пустым.", 400);
  }
  if (name.length > MAX_SETTLEMENT_NAME_LENGTH) {
    throw new GuberniaPublicationError(
      `Название населённого пункта должно быть не длиннее ${MAX_SETTLEMENT_NAME_LENGTH} символов.`,
      400,
    );
  }
  return name;
}

function validateSettlementUrl(value: unknown) {
  if (typeof value !== "string" || !isValidSettlementUrl(value)) {
    throw new GuberniaPublicationError(
      "Адрес страницы должен начинаться с «/naselennyy-punkt/» и содержать только строчные латинские буквы, цифры и одиночные дефисы.",
      400,
    );
  }
  return value;
}

/** Parses the admin-supplied "широта, долгота" string into map-ready numbers. */
function parseCoordinates(value: unknown) {
  if (typeof value !== "string") {
    throw new GuberniaPublicationError("Координаты должны быть строкой вида «широта, долгота».", 400);
  }
  const parts = value.split(",");
  if (parts.length !== 2) {
    throw new GuberniaPublicationError("Координаты должны содержать широту и долготу через запятую.", 400);
  }
  const latitudeSource = parts[0].trim();
  const longitudeSource = parts[1].trim();
  const latitude = Number(latitudeSource);
  const longitude = Number(longitudeSource);
  if (
    latitudeSource.length === 0 ||
    longitudeSource.length === 0 ||
    !Number.isFinite(latitude) ||
    !Number.isFinite(longitude)
  ) {
    throw new GuberniaPublicationError("Широта и долгота должны быть числами.", 400);
  }
  if (latitude < -90 || latitude > 90 || longitude < -180 || longitude > 180) {
    throw new GuberniaPublicationError("Координаты выходят за допустимые пределы.", 400);
  }
  return { latitude, longitude };
}

function normalizeOptionalReferenceId(value: unknown, label: string): string | null {
  if (value === undefined || value === null) return null;
  if (typeof value !== "string") {
    throw new GuberniaPublicationError(`Поле «${label}» должно быть строкой.`, 400);
  }
  const referenceId = value.trim();
  if (referenceId.length === 0) return null;
  if (referenceId.length > MAX_ID_LENGTH) {
    throw new GuberniaPublicationError(
      `Поле «${label}» должно быть не длиннее ${MAX_ID_LENGTH} символов.`,
      400,
    );
  }
  return referenceId;
}

function normalizeOptionalText(value: unknown, label: string, maxLength: number): string {
  if (value === undefined || value === null) return "";
  if (typeof value !== "string") {
    throw new GuberniaPublicationError(`Поле «${label}» должно быть строкой.`, 400);
  }
  const text = value.trim();
  if (text.length > maxLength) {
    throw new GuberniaPublicationError(
      `Поле «${label}» должно быть не длиннее ${maxLength} символов.`,
      400,
    );
  }
  return text;
}

/**
 * Resolves the uyezd/settlement pair for a post against the target province.
 * Choosing a settlement infers its district; a conflicting district or a
 * reference from another province is rejected.
 */
async function resolvePostPlacement(
  provinceId: string,
  entry: PublicationEntry,
  metadata: PostMetadataInput,
): Promise<{ uyezdId: string | null; settlementId: string | null }> {
  const settlementId = normalizeOptionalReferenceId(metadata.settlementId, "Населённый пункт");
  const requestedUyezdId = normalizeOptionalReferenceId(metadata.uyezdId, "Уезд");

  if (settlementId !== null) {
    const settlement = entry.settlements.find((candidate) => candidate.id === settlementId);
    if (!settlement) {
      throw new GuberniaPublicationError("Населённый пункт не найден в этой губернии.", 400);
    }
    if (requestedUyezdId !== null && requestedUyezdId !== settlement.uyezdId) {
      throw new GuberniaPublicationError("Уезд не соответствует выбранному населённому пункту.", 400);
    }
    return { uyezdId: settlement.uyezdId, settlementId };
  }

  if (requestedUyezdId !== null && (await getUyezd(provinceId, requestedUyezdId)) === null) {
    throw new GuberniaPublicationError("Уезд не найден в этой губернии.", 400);
  }
  return { uyezdId: requestedUyezdId, settlementId: null };
}

/**
 * Keeps the destination list newest-first: a moved post keeps its original
 * createdAt and lands before older posts but after equal timestamps.
 */
function insertPostByCreatedAt(posts: GuberniaPost[], post: GuberniaPost): GuberniaPost[] {
  const createdTime = Date.parse(post.createdAt);
  const index = posts.findIndex((candidate) => Date.parse(candidate.createdAt) < createdTime);
  if (index === -1) return [...posts, post];
  return [...posts.slice(0, index), post, ...posts.slice(index)];
}

function requirePublishedEntry(state: PublicationState, id: string) {
  const entry = state.gubernias[id];
  if (!entry.published) {
    throw new GuberniaPublicationError("Губерния не опубликована.", 404);
  }
  return entry;
}

function requirePost(entry: PublicationEntry, postId: string) {
  if (postId.length === 0 || postId.length > MAX_ID_LENGTH) {
    throw new GuberniaPublicationError("Публикация не найдена.", 404);
  }
  const post = entry.posts.find((candidate) => candidate.id === postId);
  if (!post) {
    throw new GuberniaPublicationError("Публикация не найдена.", 404);
  }
  return post;
}

// Posts and settlements stored before this feature never existed, so missing
// metadata means "not set" and the next write persists the migrated shape.
function parseStoredOptionalId(value: unknown, guberniaId: string, label: string): string | null {
  if (value === undefined || value === null) return null;
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_ID_LENGTH) {
    throw new Error(`Gubernia publication state has an invalid post ${label} for ${guberniaId}.`);
  }
  return value;
}

function parseStoredOptionalText(
  value: unknown,
  maxLength: number,
  guberniaId: string,
  label: string,
): string {
  if (value === undefined || value === null) return "";
  if (typeof value !== "string" || value.length > maxLength) {
    throw new Error(`Gubernia publication state has an invalid post ${label} for ${guberniaId}.`);
  }
  return value;
}

// Categories written under earlier settings stay readable even after the
// settings list changes, so stored values are only bounded, never matched
// against the current configuration.
function parseStoredPostCategory(value: unknown, guberniaId: string): PostCategory | null {
  if (value === undefined || value === null) return null;
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    value.length > MAX_STORED_SETTING_NAME_LENGTH
  ) {
    throw new Error(`Gubernia publication state has an invalid post category for ${guberniaId}.`);
  }
  return value;
}

// Settlements stored before this feature carry no page address; the derived
// "/naselennyy-punkt/<id>" keeps every settlement reachable until a write
// persists an explicit url.
function parseStoredSettlementUrl(
  value: unknown,
  settlementId: string,
  guberniaId: string,
): string {
  if (value === undefined || value === null) return `${SETTLEMENT_URL_PREFIX}${settlementId}`;
  if (typeof value !== "string" || !isValidSettlementUrl(value)) {
    throw new Error(`Gubernia publication state has an invalid settlement url for ${guberniaId}.`);
  }
  return value;
}

// Settlements stored before types existed carry no type and stay null; only a
// present value is bounded, never matched against the current settings.
function parseStoredSettlementType(value: unknown, guberniaId: string): string | null {
  if (value === undefined || value === null) return null;
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    value.length > MAX_STORED_SETTING_NAME_LENGTH
  ) {
    throw new Error(`Gubernia publication state has an invalid settlement type for ${guberniaId}.`);
  }
  return value;
}

function parseStoredPost(value: unknown, guberniaId: string): GuberniaPost {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Gubernia publication state has an invalid post for ${guberniaId}.`);
  }
  const post = value as Record<string, unknown>;
  if (!Object.keys(post).every((key) => STORED_POST_KEYS[key] === true)) {
    throw new Error(`Gubernia publication state has unrecognised post fields for ${guberniaId}.`);
  }
  if (
    typeof post.id !== "string" ||
    post.id.length === 0 ||
    post.id.length > MAX_ID_LENGTH ||
    typeof post.title !== "string" ||
    post.title.trim().length === 0 ||
    post.title.length > MAX_TITLE_LENGTH ||
    typeof post.createdAt !== "string" ||
    !Number.isFinite(Date.parse(post.createdAt)) ||
    typeof post.updatedAt !== "string" ||
    !Number.isFinite(Date.parse(post.updatedAt))
  ) {
    throw new Error(`Gubernia publication state has invalid post fields for ${guberniaId}.`);
  }

  let body: PostDocument;
  try {
    body = normalizePostDocument(post.body);
  } catch (error) {
    throw new Error(`Gubernia publication state has an invalid post body for ${guberniaId}.`, {
      cause: error,
    });
  }
  return {
    id: post.id,
    title: post.title,
    body,
    createdAt: post.createdAt,
    updatedAt: post.updatedAt,
    uyezdId: parseStoredOptionalId(post.uyezdId, guberniaId, "uyezd"),
    settlementId: parseStoredOptionalId(post.settlementId, guberniaId, "settlement"),
    year: parseStoredOptionalText(post.year, MAX_YEAR_LENGTH, guberniaId, "year"),
    archiveReference: parseStoredOptionalText(
      post.archiveReference,
      MAX_ARCHIVE_REFERENCE_LENGTH,
      guberniaId,
      "archive reference",
    ),
    category: parseStoredPostCategory(post.category, guberniaId),
  };
}

// Posts stored before this feature never existed, so a missing array means "no
// posts" and the next write persists the migrated shape.
function parseStoredPosts(value: unknown, guberniaId: string): GuberniaPost[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) {
    throw new Error(`Gubernia publication state has invalid posts for ${guberniaId}.`);
  }
  const ids = new Set<string>();
  return value.map((entry) => {
    const post = parseStoredPost(entry, guberniaId);
    if (ids.has(post.id)) {
      throw new Error(`Gubernia publication state contains duplicate post ${post.id}.`);
    }
    ids.add(post.id);
    return post;
  });
}

function parseStoredSettlement(value: unknown, guberniaId: string): Settlement {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Gubernia publication state has an invalid settlement for ${guberniaId}.`);
  }
  const settlement = value as Record<string, unknown>;
  if (!Object.keys(settlement).every((key) => STORED_SETTLEMENT_KEYS[key] === true)) {
    throw new Error(`Gubernia publication state has unrecognised settlement fields for ${guberniaId}.`);
  }
  if (
    typeof settlement.id !== "string" ||
    settlement.id.length === 0 ||
    settlement.id.length > MAX_ID_LENGTH ||
    typeof settlement.name !== "string" ||
    settlement.name.trim().length === 0 ||
    settlement.name.length > MAX_SETTLEMENT_NAME_LENGTH ||
    settlement.guberniaId !== guberniaId ||
    typeof settlement.uyezdId !== "string" ||
    settlement.uyezdId.length === 0 ||
    settlement.uyezdId.length > MAX_ID_LENGTH ||
    !isFiniteNumber(settlement.latitude) ||
    settlement.latitude < -90 ||
    settlement.latitude > 90 ||
    !isFiniteNumber(settlement.longitude) ||
    settlement.longitude < -180 ||
    settlement.longitude > 180 ||
    typeof settlement.createdAt !== "string" ||
    !Number.isFinite(Date.parse(settlement.createdAt))
  ) {
    throw new Error(`Gubernia publication state has invalid settlement fields for ${guberniaId}.`);
  }
  return {
    id: settlement.id,
    name: settlement.name,
    guberniaId,
    uyezdId: settlement.uyezdId,
    latitude: settlement.latitude,
    longitude: settlement.longitude,
    createdAt: settlement.createdAt,
    url: parseStoredSettlementUrl(settlement.url, settlement.id, guberniaId),
    type: parseStoredSettlementType(settlement.type, guberniaId),
  };
}

function parseStoredSettlements(value: unknown, guberniaId: string): Settlement[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) {
    throw new Error(`Gubernia publication state has invalid settlements for ${guberniaId}.`);
  }
  const ids = new Set<string>();
  return value.map((entry) => {
    const settlement = parseStoredSettlement(entry, guberniaId);
    if (ids.has(settlement.id)) {
      throw new Error(`Gubernia publication state contains duplicate settlement ${settlement.id}.`);
    }
    ids.add(settlement.id);
    return settlement;
  });
}

function seedState(collection: CanonicalCollection): PublicationState {
  return {
    version: 1,
    gubernias: Object.fromEntries(
      collection.features.map(({ properties }) => [
        properties.id,
        {
          published: properties.published === true,
          slug: properties.published === true ? properties.slug! : null,
          description: "",
          posts: [],
          settlements: [],
        },
      ]),
    ),
  };
}

function parseState(source: string, collection: CanonicalCollection): PublicationState {
  let parsed: unknown;
  try {
    parsed = JSON.parse(source);
  } catch (error) {
    throw new Error("Gubernia publication state contains invalid JSON.", { cause: error });
  }

  if (typeof parsed !== "object" || parsed === null) {
    throw new Error("Gubernia publication state is not an object.");
  }
  const candidate = parsed as { version?: unknown; gubernias?: unknown };
  if (
    candidate.version !== 1 ||
    typeof candidate.gubernias !== "object" ||
    candidate.gubernias === null ||
    Array.isArray(candidate.gubernias)
  ) {
    throw new Error("Gubernia publication state has an unsupported format.");
  }

  const entries = candidate.gubernias as Record<string, unknown>;
  const canonicalIds = new Set(collection.features.map(({ properties }) => properties.id));
  const storedIds = Object.keys(entries);
  if (storedIds.length !== canonicalIds.size || storedIds.some((id) => !canonicalIds.has(id))) {
    throw new Error("Gubernia publication state does not match the canonical feature set.");
  }

  const state: PublicationState = { version: 1, gubernias: {} };
  const slugs = new Set<string>();
  const settlementUrls = new Set<string>();
  for (const id of canonicalIds) {
    const entry = entries[id];
    if (typeof entry !== "object" || entry === null) {
      throw new Error(`Gubernia publication state has an invalid entry for ${id}.`);
    }
    const value = entry as {
      published?: unknown;
      slug?: unknown;
      description?: unknown;
      posts?: unknown;
      settlements?: unknown;
    };
    if (
      typeof value.published !== "boolean" ||
      typeof value.description !== "string" ||
      value.description.length > MAX_DESCRIPTION_LENGTH
    ) {
      throw new Error(`Gubernia publication state has invalid fields for ${id}.`);
    }
    if (value.published) {
      if (typeof value.slug !== "string" || !isValidSlug(value.slug)) {
        throw new Error(`Gubernia publication state has an invalid slug for ${id}.`);
      }
      if (slugs.has(value.slug)) {
        throw new Error(`Gubernia publication state contains duplicate slug ${value.slug}.`);
      }
      slugs.add(value.slug);
      const posts = parseStoredPosts(value.posts, id);
      const settlements = parseStoredSettlements(value.settlements, id);
      for (const settlement of settlements) {
        if (settlementUrls.has(settlement.url)) {
          throw new Error(
            `Gubernia publication state contains duplicate settlement url ${settlement.url}.`,
          );
        }
        settlementUrls.add(settlement.url);
      }
      for (const post of posts) {
        if (post.settlementId === null) continue;
        const settlement = settlements.find((candidate) => candidate.id === post.settlementId);
        if (!settlement || (post.uyezdId !== null && post.uyezdId !== settlement.uyezdId)) {
          throw new Error(`Gubernia publication state has a post with a foreign settlement for ${id}.`);
        }
      }
      state.gubernias[id] = {
        published: true,
        slug: value.slug,
        description: value.description,
        posts,
        settlements,
      };
    } else {
      if (
        value.slug !== null ||
        value.description !== "" ||
        !(value.posts === undefined || (Array.isArray(value.posts) && value.posts.length === 0)) ||
        !(
          value.settlements === undefined ||
          (Array.isArray(value.settlements) && value.settlements.length === 0)
        )
      ) {
        throw new Error(`Unpublished gubernia ${id} must not retain publication content.`);
      }
      state.gubernias[id] = { published: false, slug: null, description: "", posts: [], settlements: [] };
    }
  }

  return state;
}

async function writeState(filePath: string, state: PublicationState) {
  await mkdir(path.dirname(filePath), { recursive: true });
  const temporaryPath = `${filePath}.${process.pid}.${randomUUID()}.tmp`;
  let handle: FileHandle | undefined;
  try {
    handle = await open(temporaryPath, "wx", 0o600);
    await handle.writeFile(`${JSON.stringify(state, null, 2)}\n`, "utf8");
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

async function readState(collection: CanonicalCollection) {
  try {
    return parseState(await readFile(statePath(), "utf8"), collection);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    // The overlay file is only created by the first successful mutation, so a
    // read-only deployment still serves the ten geometry-seeded publications.
    return seedState(collection);
  }
}

function requireFeature(collection: CanonicalCollection, id: string) {
  const feature = collection.features.find(({ properties }) => properties.id === id);
  if (!feature) {
    throw new GuberniaPublicationError("Губерния не найдена.", 404);
  }
  return feature;
}

function buildPublishedGubernia(
  collection: CanonicalCollection,
  id: string,
  entry: PublicationEntry,
): PublishedGubernia {
  const feature = requireFeature(collection, id);
  return {
    id,
    name: feature.properties.name,
    slug: entry.slug!,
    description: entry.description,
    posts: entry.posts,
    settlements: entry.settlements,
  };
}

function assertUniqueSlug(state: PublicationState, slug: string, exceptId?: string) {
  const collision = Object.entries(state.gubernias).some(
    ([id, entry]) => id !== exceptId && entry.published && entry.slug === slug,
  );
  if (collision) {
    throw new GuberniaPublicationError("Этот slug уже используется другой губернией.", 409);
  }
}

function assertUniqueSettlementUrl(state: PublicationState, url: string) {
  const collision = Object.values(state.gubernias).some((entry) =>
    entry.settlements.some((settlement) => settlement.url === url),
  );
  if (collision) {
    throw new GuberniaPublicationError(
      "Этот адрес страницы уже используется другим населённым пунктом.",
      409,
    );
  }
}

export async function getGuberniasCollection(): Promise<GuberniasCollection> {
  return serialized<GuberniasCollection>(async () => {
    const collection = await canonicalCollection();
    const state = await readState(collection);
    const features: GuberniaFeature[] = collection.features.map((feature) => {
      const publication = state.gubernias[feature.properties.id];
      const { published: _published, slug: _slug, ...canonicalProperties } = feature.properties;
      return {
        type: "Feature",
        geometry: feature.geometry,
        properties: {
          ...canonicalProperties,
          published: publication.published,
          ...(publication.slug === null ? {} : { slug: publication.slug }),
        },
      };
    });
    return { type: "FeatureCollection", features };
  });
}

/** Read-only content totals; neither geometry nor post bodies leave the store. */
export async function getPublicationMetrics(): Promise<PublicationMetrics> {
  return serialized<PublicationMetrics>(async () => {
    const collection = await canonicalCollection();
    const state = await readState(collection);
    const provinces: PublicationMetricsProvince[] = [];
    let totalPublishedPosts = 0;
    let totalPublishedSettlements = 0;

    for (const feature of collection.features) {
      const entry = state.gubernias[feature.properties.id];
      if (!entry.published) continue;

      const postsCount = entry.posts.length;
      const settlementsCount = entry.settlements.length;
      totalPublishedPosts += postsCount;
      totalPublishedSettlements += settlementsCount;
      provinces.push({
        id: feature.properties.id,
        name: feature.properties.name,
        slug: entry.slug!,
        postsCount,
        settlementsCount,
      });
    }

    provinces.sort(
      (left, right) =>
        RUSSIAN_NAME_COLLATOR.compare(left.name, right.name) ||
        left.id.localeCompare(right.id),
    );

    return {
      totalHistoricalGubernias: collection.features.length,
      publishedCount: provinces.length,
      totalPublishedPosts,
      totalPublishedSettlements,
      provinces,
    };
  });
}

export async function getPublishedGuberniaBySlug(
  slug: string,
): Promise<PublishedGubernia | null> {
  return serialized<PublishedGubernia | null>(async () => {
    const collection = await canonicalCollection();
    const state = await readState(collection);
    const publication = Object.entries(state.gubernias).find(
      ([, entry]) => entry.published && entry.slug === slug,
    );
    if (!publication) return null;

    const [id, entry] = publication;
    return buildPublishedGubernia(collection, id, entry);
  });
}

/** Resolves a settlement page slug, including legacy UUID addresses, or null. */
export async function getPublishedSettlementBySlug(
  slug: string,
): Promise<PublishedSettlement | null> {
  return serialized<PublishedSettlement | null>(async () => {
    if (typeof slug !== "string" || !isValidSlug(slug)) return null;
    const collection = await canonicalCollection();
    const state = await readState(collection);
    const url = `${SETTLEMENT_URL_PREFIX}${slug}`;
    for (const [id, entry] of Object.entries(state.gubernias)) {
      if (!entry.published) continue;
      const settlement = entry.settlements.find((candidate) => candidate.url === url);
      if (settlement) {
        return { settlement, gubernia: buildPublishedGubernia(collection, id, entry) };
      }
    }
    return null;
  });
}

export async function getPublishedGeoData(): Promise<PublishedGeoData> {
  return serialized<PublishedGeoData>(async () => {
    const collection = await canonicalCollection();
    const state = await readState(collection);
    const provinces: PublishedOption[] = [];
    const settlements: Settlement[] = [];
    for (const feature of collection.features) {
      const entry = state.gubernias[feature.properties.id];
      if (!entry.published) continue;
      provinces.push({ id: feature.properties.id, name: feature.properties.name, slug: entry.slug! });
      settlements.push(...entry.settlements);
    }
    return { provinces, settlements };
  });
}

export async function publishGubernia(id: string, slugValue: unknown): Promise<PublishedGubernia> {
  return serialized<PublishedGubernia>(async () => {
    const slug = validateSlug(slugValue);
    const collection = await canonicalCollection();
    const feature = requireFeature(collection, id);
    const state = await readState(collection);
    if (state.gubernias[id].published) {
      throw new GuberniaPublicationError("Губерния уже опубликована.", 409);
    }
    assertUniqueSlug(state, slug);
    state.gubernias[id] = { published: true, slug, description: "", posts: [], settlements: [] };
    await writeState(statePath(), state);
    return {
      id,
      name: feature.properties.name,
      slug,
      description: "",
      posts: [],
      settlements: [],
    };
  });
}

export async function updateGuberniaPublication(
  id: string,
  slugValue: unknown,
  descriptionValue: unknown,
): Promise<PublishedGubernia> {
  return serialized<PublishedGubernia>(async () => {
    const slug = validateSlug(slugValue);
    const description = validateDescription(descriptionValue);
    const collection = await canonicalCollection();
    const feature = requireFeature(collection, id);
    const state = await readState(collection);
    const entry = requirePublishedEntry(state, id);
    assertUniqueSlug(state, slug, id);
    state.gubernias[id] = {
      published: true,
      slug,
      description,
      posts: entry.posts,
      settlements: entry.settlements,
    };
    await writeState(statePath(), state);
    return {
      id,
      name: feature.properties.name,
      slug,
      description,
      posts: entry.posts,
      settlements: entry.settlements,
    };
  });
}

export async function unpublishGubernia(id: string): Promise<void> {
  return serialized<void>(async () => {
    const collection = await canonicalCollection();
    requireFeature(collection, id);
    const state = await readState(collection);
    requirePublishedEntry(state, id);
    // Unpublishing clears posts and settlements: the overlay never keeps
    // content that is no longer reachable from a published province.
    state.gubernias[id] = { published: false, slug: null, description: "", posts: [], settlements: [] };
    await writeState(statePath(), state);
  });
}

export async function createGuberniaPost(
  guberniaId: string,
  titleValue: unknown,
  bodyValue: unknown,
  metadata: PostMetadataInput = {},
): Promise<GuberniaPost> {
  return serialized<GuberniaPost>(async () => {
    const title = validatePostTitle(titleValue);
    const body = validatePostBody(bodyValue);
    const collection = await canonicalCollection();
    requireFeature(collection, guberniaId);
    const state = await readState(collection);
    const entry = requirePublishedEntry(state, guberniaId);
    const placement = await resolvePostPlacement(guberniaId, entry, metadata);
    const { categories } = await getSiteSettings();
    const category = validatePostCategory(metadata.category, categories);

    const timestamp = new Date().toISOString();
    const post: GuberniaPost = {
      id: randomUUID(),
      title,
      body,
      createdAt: timestamp,
      updatedAt: timestamp,
      uyezdId: placement.uyezdId,
      settlementId: placement.settlementId,
      year: normalizeOptionalText(metadata.year, "Год", MAX_YEAR_LENGTH),
      archiveReference: normalizeOptionalText(
        metadata.archiveReference,
        "Архивный шифр",
        MAX_ARCHIVE_REFERENCE_LENGTH,
      ),
      category,
    };
    // Newest first: the public page and the anchor list share this order.
    entry.posts = [post, ...entry.posts];
    await writeState(statePath(), state);
    return post;
  });
}

export async function updateGuberniaPost(
  guberniaId: string,
  postId: string,
  titleValue: unknown,
  bodyValue: unknown,
  metadata: PostUpdateInput = {},
): Promise<GuberniaPost> {
  return serialized<GuberniaPost>(async () => {
    const title = validatePostTitle(titleValue);
    const body = validatePostBody(bodyValue);
    const collection = await canonicalCollection();
    requireFeature(collection, guberniaId);
    const state = await readState(collection);
    const entry = requirePublishedEntry(state, guberniaId);
    const post = requirePost(entry, postId);

    const targetGuberniaId =
      normalizeOptionalReferenceId(metadata.targetGuberniaId, "Губерния") ?? guberniaId;
    requireFeature(collection, targetGuberniaId);
    const targetEntry = requirePublishedEntry(state, targetGuberniaId);
    const placement = await resolvePostPlacement(targetGuberniaId, targetEntry, metadata);
    const { categories } = await getSiteSettings();
    // An unchanged category is a historical snapshot: it stays valid even if
    // the settings option was renamed or removed, but any other value must be
    // one of the current options.
    const category =
      post.category !== null && metadata.category === post.category
        ? post.category
        : validatePostCategory(metadata.category, categories);

    const updated: GuberniaPost = {
      ...post,
      title,
      body,
      uyezdId: placement.uyezdId,
      settlementId: placement.settlementId,
      year: normalizeOptionalText(metadata.year, "Год", MAX_YEAR_LENGTH),
      archiveReference: normalizeOptionalText(
        metadata.archiveReference,
        "Архивный шифр",
        MAX_ARCHIVE_REFERENCE_LENGTH,
      ),
      category,
      updatedAt: new Date().toISOString(),
    };

    if (targetGuberniaId === guberniaId) {
      entry.posts = entry.posts.map((candidate) => (candidate.id === postId ? updated : candidate));
    } else {
      // Both entries change inside one queued read-modify-write, so a failed
      // write never leaves the post in two provinces or in none.
      entry.posts = entry.posts.filter((candidate) => candidate.id !== postId);
      targetEntry.posts = insertPostByCreatedAt(targetEntry.posts, updated);
    }
    await writeState(statePath(), state);
    return updated;
  });
}

export async function deleteGuberniaPost(guberniaId: string, postId: string): Promise<void> {
  return serialized<void>(async () => {
    const collection = await canonicalCollection();
    requireFeature(collection, guberniaId);
    const state = await readState(collection);
    const entry = requirePublishedEntry(state, guberniaId);
    requirePost(entry, postId);

    entry.posts = entry.posts.filter((candidate) => candidate.id !== postId);
    await writeState(statePath(), state);
  });
}

export async function createSettlement(
  guberniaId: string,
  nameValue: unknown,
  uyezdIdValue: unknown,
  coordinatesValue: unknown,
  urlValue: unknown,
  typeValue: unknown,
): Promise<Settlement> {
  return serialized<Settlement>(async () => {
    const name = validateSettlementName(nameValue);
    const url = validateSettlementUrl(urlValue);
    const { settlementTypes } = await getSiteSettings();
    const type = validateSettlementType(typeValue, settlementTypes);
    const uyezdId = normalizeOptionalReferenceId(uyezdIdValue, "Уезд");
    if (uyezdId === null) {
      throw new GuberniaPublicationError("Уезд обязателен для населённого пункта.", 400);
    }
    const { latitude, longitude } = parseCoordinates(coordinatesValue);
    const collection = await canonicalCollection();
    requireFeature(collection, guberniaId);
    const state = await readState(collection);
    const entry = requirePublishedEntry(state, guberniaId);

    if ((await getUyezd(guberniaId, uyezdId)) === null) {
      throw new GuberniaPublicationError("Уезд не найден в этой губернии.", 400);
    }
    // The admin types "latitude, longitude"; the point-in-district test maps it
    // to GeoJSON's [longitude, latitude] order so the dot always lands on the
    // selected district, hole boundaries included.
    if (!(await isPointInUyezd(guberniaId, uyezdId, longitude, latitude))) {
      throw new GuberniaPublicationError("Точка находится за пределами выбранного уезда.", 400);
    }
    assertUniqueSettlementUrl(state, url);

    const settlement: Settlement = {
      id: randomUUID(),
      name,
      guberniaId,
      uyezdId,
      latitude,
      longitude,
      createdAt: new Date().toISOString(),
      url,
      type,
    };
    entry.settlements = [...entry.settlements, settlement];
    await writeState(statePath(), state);
    return settlement;
  });
}

/**
 * Removes one settlement while retaining its posts as province-level entries.
 * Both changes share one queued state write, so persisted posts can never point
 * at a settlement that was removed by this operation.
 */
export async function deleteSettlement(guberniaId: string, settlementId: string): Promise<void> {
  return serialized<void>(async () => {
    const collection = await canonicalCollection();
    requireFeature(collection, guberniaId);
    const state = await readState(collection);
    const entry = requirePublishedEntry(state, guberniaId);
    if (
      settlementId.length === 0 ||
      settlementId.length > MAX_ID_LENGTH ||
      !entry.settlements.some((candidate) => candidate.id === settlementId)
    ) {
      throw new GuberniaPublicationError("Населённый пункт не найден.", 404);
    }

    entry.settlements = entry.settlements.filter((candidate) => candidate.id !== settlementId);
    entry.posts = entry.posts.map((post) =>
      post.settlementId === settlementId ? { ...post, settlementId: null } : post,
    );
    await writeState(statePath(), state);
  });
}
