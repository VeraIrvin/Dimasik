import { readBackendJson, readRequiredBackendJson } from "@/lib/backend";
import type { PostDocument } from "@/lib/post-content";

/** Editorial category of a post; rows stored before categories existed stay null. */
export type PostCategory = string;

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

/** Image metadata shared by posts, province references, and settlement references. */
export type PostImage = {
  id: string;
  width: number;
  height: number;
  /** `/api/post-images/{id}/original`; resolves to a short-lived signed redirect. */
  originalUrl: string;
  /** `/api/post-images/{id}/thumbnail`; same lifetime as the original. */
  thumbnailUrl: string;
};

export type GuberniaPost = {
  id: string;
  title: string;
  body: PostDocument;
  createdAt: string;
  updatedAt: string;
  uyezdId: string | null;
  settlementId: string | null;
  year: string;
  author: string;
  archiveReference: string;
  /** Posts stored before categories existed stay null until an edit picks one. */
  category: PostCategory | null;
  /** Attached images in display order; always an array, empty when there are none. */
  images: PostImage[];
};

export type PublishedGubernia = {
  id: string;
  name: string;
  slug: string;
  /** Rich-text reference block; null when the admin stored no description. */
  description: PostDocument | null;
  /** Reference images in display order; always an array, empty when there are none. */
  images: PostImage[];
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

/** Resolves a published province by slug, or null when it is unknown or unpublished. */
export async function getPublishedGuberniaBySlug(slug: string): Promise<PublishedGubernia | null> {
  const gubernia = await readBackendJson<PublishedGubernia>(
    `/internal/gubernia/${encodeURIComponent(slug)}`,
  );
  return gubernia ? normalizePublishedGubernia(gubernia) : null;
}

/** Resolves a settlement page slug, including legacy UUID addresses, or null. */
export async function getPublishedSettlementBySlug(
  slug: string,
): Promise<PublishedSettlement | null> {
  const published = await readBackendJson<PublishedSettlement>(
    `/internal/settlement/${encodeURIComponent(slug)}`,
  );
  return published
    ? { ...published, gubernia: normalizePublishedGubernia(published.gubernia) }
    : null;
}

export function getPublishedGeoData(): Promise<PublishedGeoData> {
  return readRequiredBackendJson<PublishedGeoData>("/internal/geo");
}

/** Read-only content totals; neither geometry nor post bodies leave the store. */
export function getPublicationMetrics(): Promise<PublicationMetrics> {
  return readRequiredBackendJson<PublicationMetrics>("/internal/metrics");
}

/** Keeps callers safe while frontend and backend versions are rolled out together. */
function normalizePublishedGubernia(gubernia: PublishedGubernia): PublishedGubernia {
  return {
    ...gubernia,
    images: Array.isArray(gubernia.images) ? gubernia.images : [],
  };
}
