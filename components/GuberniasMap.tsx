'use client';

import Link from 'next/link';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent, MouseEvent as ReactMouseEvent } from 'react';
import { useRouter } from 'next/navigation';
import * as maplibregl from 'maplibre-gl';
import type {
  AroundCenterOptions,
  GeoJSONSource,
  LngLat,
  LngLatBoundsLike,
  Map as MapLibreMap,
  MapLayerMouseEvent,
  PaddingOptions,
  StyleSpecification,
} from 'maplibre-gl';
import type { Feature, FeatureCollection, MultiPolygon, Point, Polygon } from 'geojson';
import type { PublishedOption, Settlement } from '../lib/gubernia-publications';
import { districtLabelCollection } from '../lib/district-labels';
import type { SiteSettings } from '../lib/site-settings';
import SettlementEditor from '../app/guberniya/[slug]/SettlementEditor';
import SettlementListPanel from './SettlementListPanel';
import styles from './GuberniasMap.module.css';

// Next.js does not place the worker's shared module beside its bundled URL.
// The predev/prebuild hook serves both version-matched modules locally.
maplibregl.setWorkerUrl('/maplibre/maplibre-gl-worker.mjs');

const GUBERNIAS_URL = '/api/gubernias';
const HISTORICAL_CONTEXT_URL = '/data/empire-context-1897.geojson';
const EXPECTED_FEATURE_COUNT = 76;
const EXPECTED_CONTEXT_COUNT = 23;
const EXPECTED_DISTRICT_COUNT = 703;
const UYEZDS_DIRECTORY_URL = '/data/uyezds-1897';
const NAME_SUFFIX = ' губерния';
// Slug syntax shared with the server: lowercase latin, digits, single hyphens.
const SLUG_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

const CONTEXT_SOURCE_ID = 'empire-context-1897';
const CONTEXT_LAYER_ID = 'empire-context-1897';
const SOURCE_ID = 'gubernias';
const LABEL_SOURCE_ID = 'gubernia-label-points';
const FILL_LAYER_ID = 'gubernias-fill';
const BORDER_LAYER_ID = 'gubernias-border';
const PUBLISHED_FILL_LAYER_ID = 'gubernias-published-fill';
const PUBLISHED_BORDER_LAYER_ID = 'gubernias-published-border';
const LABEL_LAYER_ID = 'gubernias-labels';
const PUBLISHED_LABEL_LAYER_ID = 'gubernias-published-labels';
const SETTLEMENT_SOURCE_ID = 'settlements';
const SETTLEMENT_DOT_LAYER_ID = 'settlements-dots';
const SETTLEMENT_LABEL_LAYER_ID = 'settlements-labels';
const DISTRICT_SOURCE_ID = 'uyezds-1897';
const PROVINCE_LABEL_SOURCE_ID = 'uyezds-1897-province-label-points';
const PROVINCE_LABEL_LAYER_ID = 'uyezds-1897-province-labels';
const DISTRICT_PROVINCE_FILL_LAYER_ID = 'uyezds-1897-province-fill';
const DISTRICT_BORDER_LAYER_ID = 'uyezds-1897-district-border';
const DISTRICT_PROVINCE_BORDER_LAYER_ID = 'uyezds-1897-province-border';
const GUBERNIA_CONTEXT_COLOR = '#f1f0eb';
const GUBERNIA_FILL_COLOR = '#ecebe4';
/** Province fills of the district reconstruction: published beige, rest neutral. */
const DISTRICT_PUBLISHED_FILL_COLOR = '#e6dabc';
const DISTRICT_NEUTRAL_FILL_COLOR = '#ecebe4';
/** The `kind` values carried by the local RiStat province files. */
const PROVINCE_KIND = 'province';
const DISTRICT_KIND = 'district';
const DISTRICT_ID_PATTERN = /^uyezd-1897-\d+$/;
/** Settlement names appear after the initial overview; province names orient it immediately. */
const SETTLEMENT_LABEL_MIN_ZOOM = 3.5;
const PROVINCE_LABEL_MIN_ZOOM = 2;
const LABEL_TEXT_SIZE: ['interpolate', ['linear'], ['zoom'], number, number, number, number] =
  ['interpolate', ['linear'], ['zoom'], 3, 8, 3.8, 11];

/**
 * All geometry comes from the same local 1897 reconstruction. Governorates
 * carry borders and labels; the other administrative units remain a neutral
 * silhouette. No contemporary basemap or remote glyph service is used.
 */
const BASE_STYLE: StyleSpecification = {
  version: 8,
  sources: {},
  layers: [
    {
      id: 'canvas',
      type: 'background',
      paint: { 'background-color': '#f7f6f2' },
    },
  ],
};

type LabelCoordinates = [number, number];
type GuberniaProperties = {
  id: string;
  name: string;
  published: boolean;
  slug?: string;
  labelCoordinates: LabelCoordinates;
  labelMinZoom: number;
};
type GuberniaFeature = Feature<Polygon | MultiPolygon, GuberniaProperties>;
type GuberniasCollection = FeatureCollection<Polygon | MultiPolygon, GuberniaProperties>;
type GuberniaLabelsCollection = FeatureCollection<Point, GuberniaProperties>;
type FeatureBounds = [number, number, number, number];
type PublishedEntry = {
  id: string;
  name: string;
  slug: string;
  bounds: FeatureBounds;
};
type UnpublishedEntry = Pick<GuberniaProperties, 'id' | 'name'>;
type LoadStatus = 'loading' | 'ready' | 'error';
type CreateStatus = 'idle' | 'submitting';
type DistrictLoadStatus = 'idle' | 'loading' | 'ready' | 'error';
type OverviewMode = 'published' | 'empire';
/** The two homepage tabs: governorate geography and published settlements. */
type HomeView = 'gubernias' | 'settlements';
type SettlementProperties = { id: string; name: string; url: string };
type SettlementFeature = Feature<Point, SettlementProperties>;
type SettlementsCollection = FeatureCollection<Point, SettlementProperties>;
type DistrictProperties = {
  kind: 'province' | 'district';
  id: string;
  name: string;
  provinceId?: string;
  published?: boolean;
};
type DistrictFeature = Feature<Polygon | MultiPolygon, DistrictProperties>;
type DistrictCollection = FeatureCollection<Polygon | MultiPolygon, DistrictProperties>;
/** One interior anchor for each RiStat province polygon. */
type ProvinceLabelsCollection = FeatureCollection<Point, { id: string; name: string }>;
/** The geometry source starts empty; the 76 files fill it on the first settlement view. */
const EMPTY_DISTRICT_COLLECTION: DistrictCollection = { type: 'FeatureCollection', features: [] };
const EMPTY_PROVINCE_LABELS_COLLECTION: ProvinceLabelsCollection = {
  type: 'FeatureCollection',
  features: [],
};
type GuberniasMapProps = {
  isAdmin: boolean;
  /** Every settlement of the published governorates, with its local page URL. */
  settlements: Settlement[];
  provinces: PublishedOption[];
  /** Admin-configured lists; the settlement dialog offers `settlementTypes`. */
  settings: SiteSettings;
  /** Homepage tab deep-linked from the URL; unknown query values stay gubernias. */
  initialView: HomeView;
};

function boundsForGeometry(geometry: Polygon | MultiPolygon): FeatureBounds {
  let west = Number.POSITIVE_INFINITY;
  let south = Number.POSITIVE_INFINITY;
  let east = Number.NEGATIVE_INFINITY;
  let north = Number.NEGATIVE_INFINITY;
  let positions = 0;

  const visit = (value: unknown): void => {
    if (!Array.isArray(value)) return;
    if (
      value.length >= 2 &&
      typeof value[0] === 'number' &&
      Number.isFinite(value[0]) &&
      typeof value[1] === 'number' &&
      Number.isFinite(value[1])
    ) {
      positions += 1;
      west = Math.min(west, value[0]);
      south = Math.min(south, value[1]);
      east = Math.max(east, value[0]);
      north = Math.max(north, value[1]);
      return;
    }
    value.forEach(visit);
  };

  visit(geometry.coordinates);
  if (positions === 0) {
    throw new Error('Геометрия губернии не содержит координат.');
  }
  return [west, south, east, north];
}

/**
 * Single validation boundary: the fetched JSON is checked here and everywhere
 * else in the component works with the `GuberniaFeature` contract.
 */
function parseCollection(raw: unknown): GuberniasCollection {
  const collection = raw as { type?: unknown; features?: unknown } | null;
  if (
    typeof collection !== 'object' ||
    collection === null ||
    collection.type !== 'FeatureCollection' ||
    !Array.isArray(collection.features)
  ) {
    throw new Error('Список губерний имеет неверный формат GeoJSON.');
  }
  if (collection.features.length !== EXPECTED_FEATURE_COUNT) {
    throw new Error(
      `Ожидалось ${EXPECTED_FEATURE_COUNT} губерний, а получено ${collection.features.length}.`,
    );
  }

  const seenIds = new Set<string>();
  const seenSlugs = new Set<string>();

  const features = collection.features.map((candidate, index): GuberniaFeature => {
    const feature = candidate as
      | {
          type?: unknown;
          properties?: {
            id?: unknown;
            name?: unknown;
            published?: unknown;
            slug?: unknown;
            labelCoordinates?: unknown;
            labelMinZoom?: unknown;
          } | null;
          geometry?: { type?: unknown; coordinates?: unknown } | null;
        }
      | null;
    const properties = feature?.properties ?? null;
    const geometry = feature?.geometry ?? null;

    if (
      typeof feature !== 'object' ||
      feature === null ||
      feature.type !== 'Feature' ||
      typeof properties !== 'object' ||
      properties === null ||
      typeof properties.id !== 'string' ||
      properties.id.length === 0 ||
      typeof properties.name !== 'string' ||
      !properties.name.endsWith(NAME_SUFFIX) ||
      typeof properties.published !== 'boolean' ||
      !Array.isArray(properties.labelCoordinates) ||
      properties.labelCoordinates.length !== 2 ||
      typeof properties.labelCoordinates[0] !== 'number' ||
      !Number.isFinite(properties.labelCoordinates[0]) ||
      typeof properties.labelCoordinates[1] !== 'number' ||
      !Number.isFinite(properties.labelCoordinates[1]) ||
      typeof properties.labelMinZoom !== 'number' ||
      !Number.isFinite(properties.labelMinZoom) ||
      properties.labelMinZoom < 0 ||
      typeof geometry !== 'object' ||
      geometry === null ||
      (geometry.type !== 'Polygon' && geometry.type !== 'MultiPolygon') ||
      !Array.isArray(geometry.coordinates)
    ) {
      throw new Error(`Объект №${index + 1} не соответствует схеме губернии.`);
    }
    const slug = properties.slug === null ? undefined : properties.slug;
    if (
      (properties.published && (typeof slug !== 'string' || slug.length === 0)) ||
      (!properties.published && slug !== undefined)
    ) {
      throw new Error(`Адрес губернии «${properties.name}» не соответствует её статусу.`);
    }
    if (seenIds.has(properties.id)) {
      throw new Error(`Идентификатор губернии «${properties.name}» повторяется.`);
    }
    if (typeof slug === 'string' && seenSlugs.has(slug)) {
      throw new Error(`Адрес губернии «${properties.name}» повторяется.`);
    }
    seenIds.add(properties.id);
    if (typeof slug === 'string') seenSlugs.add(slug);

    const parsed: GuberniaFeature = {
      type: 'Feature',
      properties: {
        id: properties.id,
        name: properties.name,
        published: properties.published,
        ...(typeof slug === 'string' ? { slug } : {}),
        labelCoordinates: properties.labelCoordinates as LabelCoordinates,
        labelMinZoom: properties.labelMinZoom as number,
      },
      geometry: { type: geometry.type, coordinates: geometry.coordinates },
    } as GuberniaFeature;

    boundsForGeometry(parsed.geometry);
    return parsed;
  });

  return { type: 'FeatureCollection', features };
}

function validateContext(raw: unknown): void {
  const collection = raw as { type?: unknown; features?: unknown[] } | null;
  if (
    typeof collection !== 'object' ||
    collection === null ||
    collection.type !== 'FeatureCollection' ||
    !Array.isArray(collection.features) ||
    collection.features.length !== EXPECTED_CONTEXT_COUNT
  ) {
    throw new Error(`Исторический контекст должен содержать ${EXPECTED_CONTEXT_COUNT} области.`);
  }
  collection.features.forEach((candidate, index) => {
    const feature = candidate as
      | { type?: unknown; geometry?: { type?: unknown; coordinates?: unknown } | null }
      | null;
    if (
      feature?.type !== 'Feature' ||
      (feature.geometry?.type !== 'Polygon' && feature.geometry?.type !== 'MultiPolygon') ||
      !Array.isArray(feature.geometry.coordinates)
    ) {
      throw new Error(`Объект контекста №${index + 1} имеет неверный формат.`);
    }
  });
}

/**
 * One RiStat province file: the first feature is the province, the rest are its
 * districts. Every feature must pass this boundary, so a partial or mismatched
 * file fails the whole settlement view instead of drawing wrong boundaries.
 */
function parseDistrictFile(raw: unknown, provinceId: string): DistrictFeature[] {
  const collection = raw as { type?: unknown; features?: unknown } | null;
  if (
    typeof collection !== 'object' ||
    collection === null ||
    collection.type !== 'FeatureCollection' ||
    !Array.isArray(collection.features) ||
    collection.features.length < 2
  ) {
    throw new Error(`Файл уездов губернии ${provinceId} повреждён.`);
  }

  return collection.features.map((candidate, index): DistrictFeature => {
    const feature = candidate as
      | {
          type?: unknown;
          properties?: {
            kind?: unknown;
            id?: unknown;
            name?: unknown;
            provinceId?: unknown;
          } | null;
          geometry?: unknown;
        }
      | null;
    const properties = feature?.properties ?? null;
    const id = properties?.id;
    const name = properties?.name;
    if (
      typeof feature !== 'object' ||
      feature === null ||
      feature.type !== 'Feature' ||
      typeof properties !== 'object' ||
      properties === null ||
      typeof id !== 'string' ||
      id.length === 0 ||
      typeof name !== 'string' ||
      name.length === 0
    ) {
      throw new Error(`Объект №${index + 1} файла губернии ${provinceId} повреждён.`);
    }
    if (index === 0) {
      if (properties.kind !== PROVINCE_KIND || id !== provinceId) {
        throw new Error(`Файл уездов не соответствует губернии ${provinceId}.`);
      }
      return {
        type: 'Feature',
        properties: { kind: PROVINCE_KIND, id, name },
        geometry: parseDistrictGeometry(feature.geometry, id),
      };
    }
    const districtProvinceId = properties.provinceId;
    if (
      properties.kind !== DISTRICT_KIND ||
      typeof districtProvinceId !== 'string' ||
      districtProvinceId !== provinceId ||
      !DISTRICT_ID_PATTERN.test(id)
    ) {
      throw new Error(`Уезд «${name}» не соответствует губернии ${provinceId}.`);
    }
    return {
      type: 'Feature',
      properties: { kind: DISTRICT_KIND, id, name, provinceId: districtProvinceId },
      geometry: parseDistrictGeometry(feature.geometry, id),
    };
  });
}

/** District geometry is checked with the same rules the server applies to these files. */
function parseDistrictGeometry(value: unknown, featureId: string): Polygon | MultiPolygon {
  const geometry = value as { type?: unknown; coordinates?: unknown } | null;
  if (typeof geometry === 'object' && geometry !== null) {
    if (geometry.type === 'Polygon' && isPolygonCoordinates(geometry.coordinates)) {
      return { type: 'Polygon', coordinates: geometry.coordinates };
    }
    if (
      geometry.type === 'MultiPolygon' &&
      Array.isArray(geometry.coordinates) &&
      geometry.coordinates.length > 0 &&
      geometry.coordinates.every(isPolygonCoordinates)
    ) {
      return { type: 'MultiPolygon', coordinates: geometry.coordinates };
    }
  }
  throw new Error(`Геометрия объекта ${featureId} повреждена.`);
}

/** GeoJSON polygon coordinates: rings of at least four finite [lng, lat] positions. */
function isPolygonCoordinates(value: unknown): value is [number, number][][] {
  return (
    Array.isArray(value) &&
    value.length > 0 &&
    value.every(
      (ring) =>
        Array.isArray(ring) &&
        ring.length >= 4 &&
        ring.every(
          (position) =>
            Array.isArray(position) &&
            position.length >= 2 &&
            typeof position[0] === 'number' &&
            Number.isFinite(position[0]) &&
            typeof position[1] === 'number' &&
            Number.isFinite(position[1]),
        ),
    )
  );
}

/** One merged collection: 76 provinces plus their districts, no reused id. */
function mergeDistrictFiles(files: DistrictFeature[][]): DistrictFeature[] {
  const merged: DistrictFeature[] = [];
  const seenIds = new Set<string>();
  for (const file of files) {
    for (const feature of file) {
      const id = feature.properties.id;
      if (seenIds.has(id)) {
        throw new Error(`Идентификатор ${id} повторяется в файлах уездов.`);
      }
      seenIds.add(id);
      merged.push(feature);
    }
  }
  return merged;
}

/** The reconstruction is known to carry 76 provinces and 703 districts in total. */
function assertDistrictIntegrity(features: DistrictFeature[]): void {
  let provinces = 0;
  for (const feature of features) {
    if (feature.properties.kind === PROVINCE_KIND) provinces += 1;
  }
  if (provinces !== EXPECTED_FEATURE_COUNT) {
    throw new Error(`Ожидалось ${EXPECTED_FEATURE_COUNT} губерний, а загружено ${provinces}.`);
  }
  const districts = features.length - provinces;
  if (districts !== EXPECTED_DISTRICT_COUNT) {
    throw new Error(`Ожидалось ${EXPECTED_DISTRICT_COUNT} уездов, а загружено ${districts}.`);
  }
}

/** Province shading follows the publication list; district features pass through as loaded. */
function withPublishedFlags(
  features: DistrictFeature[],
  publishedIds: Set<string>,
): DistrictCollection {
  return {
    type: 'FeatureCollection',
    features: features.map((feature) =>
      feature.properties.kind === PROVINCE_KIND
        ? {
            ...feature,
            properties: {
              ...feature.properties,
              published: publishedIds.has(feature.properties.id),
            },
          }
        : feature,
    ),
  };
}

function createLabelCollection(collection: GuberniasCollection): GuberniaLabelsCollection {
  return {
    type: 'FeatureCollection',
    features: collection.features.map((feature) => ({
      type: 'Feature',
      properties: feature.properties,
      geometry: {
        type: 'Point',
        coordinates: feature.properties.labelCoordinates,
      },
    })),
  };
}

/**
 * Homepage settlement entries carry their origin in the query string so the
 * settlement page can offer the matching way back. The store keeps the local
 * page address itself query-free, so the flag is appended here.
 */
function settlementHref(url: string): string {
  return `${url}?from=settlements`;
}

/**
 * One point per published settlement. The local page address comes from the
 * store as provided; the map only navigates to it.
 */
function createSettlementCollection(settlements: Settlement[]): SettlementsCollection {
  return {
    type: 'FeatureCollection',
    features: settlements.map((settlement): SettlementFeature => ({
      type: 'Feature',
      id: settlement.id,
      properties: {
        id: settlement.id,
        name: settlement.name,
        url: settlement.url,
      },
      geometry: {
        type: 'Point',
        coordinates: [settlement.longitude, settlement.latitude],
      },
    })),
  };
}

function mergeBounds(all: FeatureBounds[]): FeatureBounds {
  return all.reduce<FeatureBounds>(
    (combined, current) => [
      Math.min(combined[0], current[0]),
      Math.min(combined[1], current[1]),
      Math.max(combined[2], current[2]),
      Math.max(combined[3], current[3]),
    ],
    [
      Number.POSITIVE_INFINITY,
      Number.POSITIVE_INFINITY,
      Number.NEGATIVE_INFINITY,
      Number.NEGATIVE_INFINITY,
    ],
  );
}

function expandedMaxBounds([west, south, east, north]: FeatureBounds): LngLatBoundsLike {
  // Leave room for the viewport's aspect ratio and the floating list.
  // Narrow maxBounds force MapLibre to zoom in and clip edge provinces.
  const horizontal = Math.max((east - west) * 0.7, 1);
  const vertical = Math.max((north - south) * 0.7, 1);
  return [
    [Math.max(-180, west - horizontal), Math.max(-85, south - vertical)],
    [Math.min(180, east + horizontal), Math.min(85, north + vertical)],
  ];
}

/**
 * Fit padding per breakpoint. Both tabs keep a floating overlay list on the
 * left — governorates or settlements — so the fitted bounds clear it on every
 * size, and the settlement tab reuses the governorate padding unchanged.
 */
function fitPadding(width: number) {
  if (width <= 560) {
    // Clears the two-column list at the top and the caption above the zoom control.
    return { top: 212, right: 12, bottom: 112, left: 12 };
  }
  if (width <= 960) {
    return { top: 64, right: 40, bottom: 56, left: 324 };
  }
  return { top: 64, right: 64, bottom: 64, left: 340 };
}

function paddingBreakpoint(width: number) {
  if (width <= 560) return 'mobile';
  if (width <= 960) return 'compact';
  return 'wide';
}

/** Floating-point slack for comparing the camera against the zoom-out floor. */
const ZOOM_EPSILON = 0.02;
/** Slack in degrees for treating the camera center as already on the focus. */
const FOCUS_EPSILON = 0.05;

type OverviewFocus = { center: LngLat; zoom: number; floor: number | undefined };

/**
 * The camera the overview toggle fits for `mode`, plus the deepest zoom at
 * which that camera can stay centered. Every correction below reuses these two
 * numbers, so padding, breakpoint, and extent changes stay consistent with the
 * overview buttons.
 */
function overviewFocus(
  map: MapLibreMap,
  bounds: { published: FeatureBounds; all: FeatureBounds },
  mode: OverviewMode,
  padding: PaddingOptions,
): OverviewFocus | undefined {
  const [west, south, east, north] = mode === 'published' ? bounds.published : bounds.all;
  const camera = map.cameraForBounds(
    [
      [west, south],
      [east, north],
    ],
    { padding, absolutePadding: true },
  );
  if (!camera?.center || typeof camera.zoom !== 'number') return undefined;
  const center = maplibregl.LngLat.convert(camera.center);
  return { center, zoom: camera.zoom, floor: zoomOutFloor(map, center, padding) };
}

/**
 * Deepest zoom that still lets the camera keep `focus` centered inside the max
 * bounds. MapLibre's own clamp stops zooming out only once the whole empire
 * fills the usable viewport, which welds the camera center to the middle of the
 * empire — empty sea north of the published governorates — so zooming back in
 * there magnified that emptiness. This floor is the zoom at which the focus can
 * still be centered, which keeps the widest fully zoomed-out view on the focus.
 */
function zoomOutFloor(
  map: MapLibreMap,
  focus: LngLat,
  padding: PaddingOptions,
): number | undefined {
  const box = map.getMaxBounds();
  if (!box) return undefined;
  const point = maplibregl.MercatorCoordinate.fromLngLat(focus);
  const northWest = maplibregl.MercatorCoordinate.fromLngLat({
    lng: box.getWest(),
    lat: box.getNorth(),
  });
  const southEast = maplibregl.MercatorCoordinate.fromLngLat({
    lng: box.getEast(),
    lat: box.getSouth(),
  });
  // Mirror the focus-to-edge distance onto the opposite side: the deepest zoom
  // that keeps the focus centered while the view still fits inside maxBounds.
  const halfX = Math.min(point.x - northWest.x, southEast.x - point.x);
  const halfY = Math.min(point.y - northWest.y, southEast.y - point.y);
  if (halfX <= 0 || halfY <= 0) return undefined;
  const focusWindow: LngLatBoundsLike = [
    new maplibregl.MercatorCoordinate(point.x - halfX, point.y - halfY).toLngLat(),
    new maplibregl.MercatorCoordinate(point.x + halfX, point.y + halfY).toLngLat(),
  ];
  return map.cameraForBounds(focusWindow, { padding, absolutePadding: true })?.zoom;
}

/**
 * Raises the zoom-out floor to `floor`. Lowering it is invisible at any time;
 * raising it waits for the camera to settle while a transition is under way, so
 * an animated overview switch keeps its glide instead of snapping to the floor.
 */
function settleZoomOutFloor(map: MapLibreMap, floor: number) {
  if (floor <= map.getMinZoom() || map.getZoom() >= floor) {
    map.setMinZoom(floor);
    return;
  }
  map.once('moveend', () => {
    // An interrupted transition can settle below the floor; the next overview
    // fit or floor recovery re-applies it then.
    if (map.getZoom() >= floor - ZOOM_EPSILON) map.setMinZoom(floor);
  });
}

/** Keeps the zoom-out floor on the overview focus of `mode`; returns that overview's fitted zoom. */
function applyZoomOutFloor(
  map: MapLibreMap,
  bounds: { published: FeatureBounds; all: FeatureBounds },
  mode: OverviewMode,
  padding: PaddingOptions,
): number | undefined {
  const focus = overviewFocus(map, bounds, mode, padding);
  if (!focus) return undefined;
  if (focus.floor !== undefined) settleZoomOutFloor(map, focus.floor);
  return focus.zoom;
}

/**
 * Anchors wheel and pinch zoom on the map center — the overview focus — while
 * the camera is zoomed out past that overview, so a zoom-in from there magnifies
 * the published set instead of whatever happens to sit under the pointer.
 * Ordinary zoom levels keep MapLibre's pointer anchor. The anchor is settable
 * only through `enable()`, which ignores an already enabled scroll handler, so
 * the scroll handler is cycled; both calls are synchronous, leaving no gap in
 * which a wheel event could be dropped. The touch shim skips its disabled
 * rotation handler, so pinch-to-rotate stays off.
 */
function setFloorZoomAnchor(map: MapLibreMap, onFocus: boolean) {
  const around: AroundCenterOptions | undefined = onFocus ? { around: 'center' } : undefined;
  map.scrollZoom.disable();
  map.scrollZoom.enable(around);
  map.touchZoomRotate.enable(around);
}

function setFeatureHover(map: MapLibreMap, id: string, value: boolean) {
  map.setFeatureState({ source: SOURCE_ID, id }, { hover: value });
  map.setFeatureState({ source: LABEL_SOURCE_ID, id }, { hover: value });
}

/**
 * A layer that a style error kept off the map must not take the rest of the
 * wiring down with it: every visibility write checks the layer first.
 */
function setLayerVisibility(map: MapLibreMap, id: string, visible: boolean) {
  if (!map.getLayer(id)) return;
  map.setLayoutProperty(id, 'visibility', visible ? 'visible' : 'none');
}

/** «10 доступных губерний», «1 доступная губерния», «22 доступные губернии». */
function publishedCountLabel(count: number): string {
  const mod10 = count % 10;
  const mod100 = count % 100;
  if (mod10 === 1 && mod100 !== 11) return `${count} доступная губерния`;
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) {
    return `${count} доступные губернии`;
  }
  return `${count} доступных губерний`;
}

/** «1 населённый пункт», «3 населённых пункта», «12 населённых пунктов». */
function settlementCountLabel(count: number): string {
  const mod10 = count % 10;
  const mod100 = count % 100;
  if (mod10 === 1 && mod100 !== 11) return `${count} населённый пункт`;
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) {
    return `${count} населённых пункта`;
  }
  return `${count} населённых пунктов`;
}

/**
 * Polygon and point sources share the merged API payload so publication
 * changes can be applied without rebuilding the map or moving its camera.
 */
function addGuberniaSourceAndLayers(
  map: MapLibreMap,
  collection: GuberniasCollection,
  labelCollection: GuberniaLabelsCollection,
) {
  // Other 1897 provinces establish historical geography without interactive
  // boundaries, labels, or a modern-day basemap.
  map.addSource(CONTEXT_SOURCE_ID, { type: 'geojson', data: HISTORICAL_CONTEXT_URL });
  map.addLayer({
    id: CONTEXT_LAYER_ID,
    type: 'fill',
    source: CONTEXT_SOURCE_ID,
    paint: { 'fill-color': GUBERNIA_CONTEXT_COLOR },
  });

  map.addSource(SOURCE_ID, { type: 'geojson', data: collection, promoteId: 'id' });
  map.addSource(LABEL_SOURCE_ID, {
    type: 'geojson',
    data: labelCollection,
    promoteId: 'id',
  });

  // Neutral outline for all 76 governorates keeps unpublished geography visible
  // without presenting it as interactive.
  map.addLayer({
    id: FILL_LAYER_ID,
    type: 'fill',
    source: SOURCE_ID,
    paint: { 'fill-color': GUBERNIA_FILL_COLOR },
  });

  map.addLayer({
    id: BORDER_LAYER_ID,
    type: 'line',
    source: SOURCE_ID,
    paint: {
      'line-color': '#d5d1c7',
      'line-width': 0.8,
      'line-opacity': 0.9,
    },
  });

  // Published governorates are distinct at rest, become stronger on hover,
  // and are the only features that react to input.
  map.addLayer({
    id: PUBLISHED_FILL_LAYER_ID,
    type: 'fill',
    source: SOURCE_ID,
    filter: ['==', ['get', 'published'], true],
    paint: {
      'fill-color': [
        'case',
        ['boolean', ['feature-state', 'hover'], false],
        '#d8c393',
        '#e6dabc',
      ],
      'fill-color-transition': { duration: 160, delay: 0 },
    },
  });

  map.addLayer({
    id: PUBLISHED_BORDER_LAYER_ID,
    type: 'line',
    source: SOURCE_ID,
    filter: ['==', ['get', 'published'], true],
    paint: {
      'line-color': [
        'case',
        ['boolean', ['feature-state', 'hover'], false],
        '#6f5a2b',
        '#a08a51',
      ],
      'line-width': [
        'case',
        ['boolean', ['feature-state', 'hover'], false],
        2.2,
        1.3,
      ],
      'line-opacity': 0.95,
      'line-color-transition': { duration: 160, delay: 0 },
      'line-width-transition': { duration: 160, delay: 0 },
    },
  });

  // Neutral names retain their scale threshold so the empire-wide view stays
  // uncluttered. All names use the same font size and interior point source.
  map.addLayer({
    id: LABEL_LAYER_ID,
    type: 'symbol',
    source: LABEL_SOURCE_ID,
    filter: [
      'all',
      ['==', ['get', 'published'], false],
      ['<=', ['get', 'labelMinZoom'], 0],
    ],
    layout: {
      'text-field': ['slice', ['get', 'name'], 0, -NAME_SUFFIX.length],
      'text-font': ['Arial', 'Helvetica', 'sans-serif'],
      'text-size': LABEL_TEXT_SIZE,
      'text-letter-spacing': 0.02,
      'text-max-width': 50,
      'text-padding': 4,
      'text-anchor': 'center',
      'text-justify': 'center',
      'text-allow-overlap': false,
      'text-ignore-placement': false,
    },
    paint: {
      'text-color': '#8f8b80',
      'text-halo-color': '#f7f6f2',
      'text-halo-width': 1.2,
      'text-halo-blur': 0.2,
    },
  });

  // Published names remain centered and legible at the published overview.
  // Text may extend past a polygon edge; never split the name.
  map.addLayer({
    id: PUBLISHED_LABEL_LAYER_ID,
    type: 'symbol',
    source: LABEL_SOURCE_ID,
    filter: ['==', ['get', 'published'], true],
    layout: {
      'text-field': ['slice', ['get', 'name'], 0, -NAME_SUFFIX.length],
      'text-font': ['Arial', 'Helvetica', 'sans-serif'],
      'text-size': LABEL_TEXT_SIZE,
      // Small neighboring governorates need a few pixels of separation while
      // the anchors remain near their interior visual centers.
      'text-offset': [
        'match',
        ['get', 'id'],
        'vladimir', ['literal', [-1.7, 0]],
        'nizhny-novgorod', ['literal', [1.7, 0]],
        'kaluga', ['literal', [-1, 0]],
        'tula', ['literal', [1, 0]],
        ['literal', [0, 0]],
      ],
      'text-letter-spacing': 0.02,
      'text-max-width': 50,
      'text-padding': 4,
      'text-anchor': 'center',
      'text-justify': 'center',
      'text-allow-overlap': true,
      'text-ignore-placement': false,
    },
    paint: {
      'text-color': [
        'case',
        ['boolean', ['feature-state', 'hover'], false],
        '#272622',
        '#3f3b34',
      ],
      'text-color-transition': { duration: 160, delay: 0 },
      'text-halo-color': '#f7f6f2',
      'text-halo-width': 1.2,
      'text-halo-blur': 0.2,
    },
  });

  // The RiStat district reconstruction replaces the governorate presentation in
  // the settlement tab: one source feeds neutral province fills, beige published
  // provinces, and the thin district outlines above both. It starts empty and is
  // filled from the 76 local files on the first settlement view, including a
  // deep link straight into that tab.
  map.addSource(DISTRICT_SOURCE_ID, { type: 'geojson', data: EMPTY_DISTRICT_COLLECTION });

  map.addLayer({
    id: DISTRICT_PROVINCE_FILL_LAYER_ID,
    type: 'fill',
    source: DISTRICT_SOURCE_ID,
    filter: ['==', ['get', 'kind'], PROVINCE_KIND],
    layout: { visibility: 'none' },
    paint: {
      'fill-color': [
        'case',
        ['boolean', ['get', 'published'], false],
        DISTRICT_PUBLISHED_FILL_COLOR,
        DISTRICT_NEUTRAL_FILL_COLOR,
      ],
      'fill-color-transition': { duration: 160, delay: 0 },
    },
  });

  map.addLayer({
    id: DISTRICT_BORDER_LAYER_ID,
    type: 'line',
    source: DISTRICT_SOURCE_ID,
    filter: ['==', ['get', 'kind'], DISTRICT_KIND],
    layout: { visibility: 'none' },
    paint: {
      'line-color': '#d7d3c7',
      'line-width': 0.6,
      'line-opacity': 0.8,
    },
  });

  map.addLayer({
    id: DISTRICT_PROVINCE_BORDER_LAYER_ID,
    type: 'line',
    source: DISTRICT_SOURCE_ID,
    filter: ['==', ['get', 'kind'], PROVINCE_KIND],
    layout: { visibility: 'none' },
    paint: {
      'line-color': [
        'case',
        ['boolean', ['get', 'published'], false],
        '#a08a51',
        '#cfcabd',
      ],
      'line-width': [
        'case',
        ['boolean', ['get', 'published'], false],
        1.3,
        0.9,
      ],
      'line-opacity': 0.9,
      'line-color-transition': { duration: 160, delay: 0 },
    },
  });

  // Quiet province names orient the settlement tab from its initial overview.
  // This layer stays below the settlement layers; MapLibre places later symbol
  // layers first, so settlement names retain collision priority.
  map.addSource(PROVINCE_LABEL_SOURCE_ID, {
    type: 'geojson',
    data: EMPTY_PROVINCE_LABELS_COLLECTION,
  });

  map.addLayer({
    id: PROVINCE_LABEL_LAYER_ID,
    type: 'symbol',
    source: PROVINCE_LABEL_SOURCE_ID,
    minzoom: PROVINCE_LABEL_MIN_ZOOM,
    layout: {
      visibility: 'none',
      'text-field': ['slice', ['get', 'name'], 0, -NAME_SUFFIX.length],
      'text-font': ['Arial', 'Helvetica', 'sans-serif'],
      'text-size': ['interpolate', ['linear'], ['zoom'], 2, 8, 6, 10],
      'text-letter-spacing': 0.02,
      'text-max-width': 14,
      'text-padding': 3,
      'text-anchor': 'center',
      'text-allow-overlap': false,
      'text-ignore-placement': false,
    },
    paint: {
      'text-color': '#68665f',
      'text-opacity': 0.42,
      'text-halo-color': '#f7f6f2',
      'text-halo-width': 1,
      'text-halo-blur': 0.25,
    },
  });

  // Published settlement points live on their own source so the settlement tab
  // can show them without any governorate boundary. Both layers start hidden;
  // the tab switch decides which presentation is on screen.
  map.addSource(SETTLEMENT_SOURCE_ID, {
    type: 'geojson',
    data: createSettlementCollection([]),
    promoteId: 'id',
  });

  map.addLayer({
    id: SETTLEMENT_DOT_LAYER_ID,
    type: 'circle',
    source: SETTLEMENT_SOURCE_ID,
    layout: { visibility: 'none' },
    paint: {
      // An expression may carry only one zoom-based "interpolate", so the
      // hover size is applied per stop instead of wrapping the zoom curve.
      'circle-radius': [
        'interpolate',
        ['linear'],
        ['zoom'],
        2,
        ['case', ['boolean', ['feature-state', 'hover'], false], 5, 3.4],
        5,
        ['case', ['boolean', ['feature-state', 'hover'], false], 6.6, 4.8],
        8,
        ['case', ['boolean', ['feature-state', 'hover'], false], 8.2, 6],
      ],
      'circle-color': [
        'case',
        ['boolean', ['feature-state', 'hover'], false],
        '#8a6d2f',
        '#a08a51',
      ],
      'circle-stroke-color': '#fffdf7',
      'circle-stroke-width': 1.4,
      'circle-radius-transition': { duration: 140, delay: 0 },
      'circle-color-transition': { duration: 140, delay: 0 },
    },
  });

  map.addLayer({
    id: SETTLEMENT_LABEL_LAYER_ID,
    type: 'symbol',
    source: SETTLEMENT_SOURCE_ID,
    layout: {
      visibility: 'none',
      'text-field': ['get', 'name'],
      'text-font': ['Arial', 'Helvetica', 'sans-serif'],
      'text-size': ['interpolate', ['linear'], ['zoom'], 3.5, 10.5, 7, 12.5],
      'text-max-width': 10,
      'text-padding': 2,
      'text-anchor': 'top',
      'text-offset': [0, 0.7],
      'text-allow-overlap': false,
      'text-ignore-placement': false,
      'text-optional': true,
    },
    paint: {
      'text-color': '#3f3a2f',
      'text-halo-color': '#f7f6f2',
      'text-halo-width': 1.3,
      'text-halo-blur': 0.2,
    },
  });
}

export default function GuberniasMap({
  isAdmin,
  settlements,
  provinces,
  settings,
  initialView,
}: GuberniasMapProps) {
  const router = useRouter();
  const containerRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<MapLibreMap | null>(null);
  const collectionRef = useRef<GuberniasCollection | null>(null);
  const publishedIdsRef = useRef<Set<string>>(new Set());
  const hoveredIdRef = useRef<string | null>(null);
  const hoverOriginRef = useRef<'list' | 'map' | null>(null);
  const overviewRef = useRef<OverviewMode>('published');
  const settlementOverviewRef = useRef<OverviewMode>('published');
  const viewRef = useRef<HomeView>(initialView);
  const settlementHoverRef = useRef<string | null>(null);
  const settlementHoverOriginRef = useRef<'list' | 'map' | null>(null);
  const settlementDataRef = useRef<SettlementsCollection>(createSettlementCollection([]));
  /** Validated RiStat features, kept across tab switches; `null` until loaded. */
  const districtFeaturesRef = useRef<DistrictFeature[] | null>(null);
  /** Province interior points are computed once with the validated RiStat geometry. */
  const provinceLabelsRef = useRef<ProvinceLabelsCollection | null>(null);
  const districtRequestRef = useRef<AbortController | null>(null);
  const labelFilterRef = useRef<(() => void) | null>(null);
  const viewSyncRef = useRef<(() => void) | null>(null);
  const boundsRef = useRef<{ published: FeatureBounds; all: FeatureBounds } | null>(null);
  /** Fitted zoom of the current overview; below it wheel and pinch aim at its focus. */
  const anchorBandRef = useRef<number | null>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const settlementDialogRef = useRef<HTMLDialogElement>(null);

  const [entries, setEntries] = useState<PublishedEntry[]>([]);
  const [unpublished, setUnpublished] = useState<UnpublishedEntry[]>([]);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [settlementHoveredId, setSettlementHoveredId] = useState<string | null>(null);
  const [overview, setOverview] = useState<OverviewMode>('published');
  const [settlementOverview, setSettlementOverview] = useState<OverviewMode>('published');
  const [view, setView] = useState<HomeView>(initialView);
  const [status, setStatus] = useState<LoadStatus>('loading');
  const [districtStatus, setDistrictStatus] = useState<DistrictLoadStatus>('idle');
  const [districtError, setDistrictError] = useState('');
  const [errorMessage, setErrorMessage] = useState('');
  const [isDialogOpen, setIsDialogOpen] = useState(false);
  const [isSettlementDialogOpen, setIsSettlementDialogOpen] = useState(false);
  const [selectedId, setSelectedId] = useState('');
  const [slug, setSlug] = useState('');
  const [createStatus, setCreateStatus] = useState<CreateStatus>('idle');
  const [createError, setCreateError] = useState('');

  /** Point features keep the store order; ids stay stable across updates. */
  const settlementData = useMemo(() => createSettlementCollection(settlements), [settlements]);

  /**
   * The overview the camera follows. Each tab keeps its own extent choice:
   * the button in one tab never changes what the other tab returns to.
   */
  const cameraOverview = useCallback(
    (): OverviewMode =>
      viewRef.current === 'settlements' ? settlementOverviewRef.current : overviewRef.current,
    [],
  );

  /**
   * Settlement hover is shared by the map dots and the settlement list. The
   * origin lets a list clear never hijack a hover the map owns.
   */
  const setSettlementHover = useCallback((id: string | null, origin: 'list' | 'map') => {
    const current = settlementHoverRef.current;
    if (id === null && settlementHoverOriginRef.current !== origin) return;
    if (current === id) {
      if (id !== null) settlementHoverOriginRef.current = origin;
      return;
    }
    const map = mapRef.current;
    if (map?.getSource(SETTLEMENT_SOURCE_ID)) {
      if (current !== null) {
        map.setFeatureState({ source: SETTLEMENT_SOURCE_ID, id: current }, { hover: false });
      }
      if (id !== null) {
        map.setFeatureState({ source: SETTLEMENT_SOURCE_ID, id }, { hover: true });
      }
    }
    settlementHoverRef.current = id;
    settlementHoverOriginRef.current = id === null ? null : origin;
    setSettlementHoveredId(id);
  }, []);

  /** Stable list-side entry point: every list hover is list-origin. */
  const hoverSettlementFromList = useCallback(
    (id: string | null) => setSettlementHover(id, 'list'),
    [setSettlementHover],
  );

  // Markers follow the `settlements` prop, so a settlement created elsewhere
  // shows up after router.refresh() without recreating the map.
  useEffect(() => {
    settlementDataRef.current = settlementData;
    const map = mapRef.current;
    const source = map?.getSource(SETTLEMENT_SOURCE_ID) as GeoJSONSource | undefined;
    if (!source) return;
    source.setData(settlementData);
    const hovered = settlementHoverRef.current;
    if (hovered !== null && !settlements.some((settlement) => settlement.id === hovered)) {
      // The replaced source drops the feature states of removed points.
      settlementHoverRef.current = null;
      settlementHoverOriginRef.current = null;
      setSettlementHoveredId(null);
    }
  }, [settlementData, settlements]);

  /** The merged API payload is the single publication source of truth. */
  const fetchCollection = useCallback(async (signal?: AbortSignal) => {
    const response = await fetch(GUBERNIAS_URL, { cache: 'no-store', signal });
    if (!response.ok) {
      throw new Error(`Не удалось загрузить список губерний (HTTP ${response.status}).`);
    }
    return parseCollection(await response.json());
  }, []);

  /** Re-shades the district provinces after a publication change. */
  const syncDistrictPublished = useCallback(() => {
    const features = districtFeaturesRef.current;
    const source = mapRef.current?.getSource(DISTRICT_SOURCE_ID) as GeoJSONSource | undefined;
    if (!features || !source) return;
    source.setData(withPublishedFlags(features, publishedIdsRef.current));
  }, []);

  /**
   * Loads the 76 local RiStat files for the settlement tab: district outlines,
   * province fills, and the muted province names derived from the same
   * polygons. Both the validated features and the in-flight request are
   * remembered, so later switches reuse the list and a failed load is retried
   * on the next one.
   */
  const loadDistricts = useCallback(async () => {
    const collection = collectionRef.current;
    if (!collection || districtFeaturesRef.current || districtRequestRef.current) return;
    const controller = new AbortController();
    districtRequestRef.current = controller;
    setDistrictStatus('loading');
    setDistrictError('');
    try {
      const files = await Promise.all(
        collection.features.map(async (feature) => {
          const provinceId = feature.properties.id;
          const response = await fetch(
            `${UYEZDS_DIRECTORY_URL}/${encodeURIComponent(provinceId)}.geojson`,
            { signal: controller.signal },
          );
          if (!response.ok) {
            throw new Error(
              `Не удалось загрузить уезды губернии «${feature.properties.name}» (HTTP ${response.status}).`,
            );
          }
          return parseDistrictFile(await response.json(), provinceId);
        }),
      );
      const features = mergeDistrictFiles(files);
      assertDistrictIntegrity(features);
      districtFeaturesRef.current = features;
      const labels = districtLabelCollection(
        features.filter((feature) => feature.properties.kind === PROVINCE_KIND),
      );
      provinceLabelsRef.current = labels;
      const source = mapRef.current?.getSource(DISTRICT_SOURCE_ID) as GeoJSONSource | undefined;
      source?.setData(withPublishedFlags(features, publishedIdsRef.current));
      const labelSource = mapRef.current?.getSource(PROVINCE_LABEL_SOURCE_ID) as
        | GeoJSONSource
        | undefined;
      labelSource?.setData(labels);
      viewSyncRef.current?.();
      setDistrictStatus('ready');
    } catch (error) {
      if (controller.signal.aborted) return;
      districtFeaturesRef.current = null;
      provinceLabelsRef.current = null;
      const labelSource = mapRef.current?.getSource(PROVINCE_LABEL_SOURCE_ID) as
        | GeoJSONSource
        | undefined;
      labelSource?.setData(EMPTY_PROVINCE_LABELS_COLLECTION);
      viewSyncRef.current?.();
      setDistrictError(
        error instanceof Error ? error.message : 'Не удалось загрузить уезды 1897 года.',
      );
      setDistrictStatus('error');
    } finally {
      if (districtRequestRef.current === controller) districtRequestRef.current = null;
    }
  }, []);

  // A deep link can open the homepage already in the settlement view: run the
  // same load the tab switch triggers as soon as the governorate payload and
  // the map are ready, so the province names and outlines need no manual tab
  // switch. The in-flight guard makes this call a no-op when it already ran.
  useEffect(() => {
    if (view === 'settlements' && status === 'ready') void loadDistricts();
  }, [loadDistricts, status, view]);

  /**
   * Applies one authoritative collection to every derived surface: list,
   * unpublished options, stored bounds, and both MapLibre sources. The map
   * camera, history, and hover wiring stay untouched.
   */
  const applyCollection = useCallback((collection: GuberniasCollection) => {
    const nextEntries = collection.features
      .filter((feature) => feature.properties.published)
      .map((feature): PublishedEntry => ({
        id: feature.properties.id,
        name: feature.properties.name,
        slug: feature.properties.slug as string,
        bounds: boundsForGeometry(feature.geometry),
      }))
      .sort((left, right) => left.name.localeCompare(right.name, 'ru'));
    const nextUnpublished = collection.features
      .filter((feature) => !feature.properties.published)
      .map((feature): UnpublishedEntry => ({
        id: feature.properties.id,
        name: feature.properties.name,
      }))
      .sort((left, right) => left.name.localeCompare(right.name, 'ru'));
    const allBounds = mergeBounds(
      collection.features.map((feature) => boundsForGeometry(feature.geometry)),
    );
    const publishedBounds =
      nextEntries.length > 0 ? mergeBounds(nextEntries.map((entry) => entry.bounds)) : allBounds;

    const nextBounds = { published: publishedBounds, all: allBounds };
    collectionRef.current = collection;
    boundsRef.current = nextBounds;
    publishedIdsRef.current = new Set(nextEntries.map((entry) => entry.id));
    syncDistrictPublished();
    setEntries(nextEntries);
    setUnpublished(nextUnpublished);

    const map = mapRef.current;
    const source = map?.getSource(SOURCE_ID) as GeoJSONSource | undefined;
    if (!map || !source) return;
    source.setData(collection);
    const labelSource = map.getSource(LABEL_SOURCE_ID) as GeoJSONSource | undefined;
    labelSource?.setData(createLabelCollection(collection));
    // A data swap clears hover rendering; re-assert the shared hover state.
    const hovered = hoveredIdRef.current;
    if (hovered !== null && publishedIdsRef.current.has(hovered)) {
      setFeatureHover(map, hovered, true);
    }
    labelFilterRef.current?.();
    // A publication moves the focus; keep the zoom-out floor with it so the
    // fully zoomed-out camera keeps landing on the active tab's overview.
    anchorBandRef.current =
      applyZoomOutFloor(
        map,
        nextBounds,
        cameraOverview(),
        fitPadding(containerRef.current?.clientWidth ?? window.innerWidth),
      ) ?? null;
  }, [cameraOverview, syncDistrictPublished]);

  /**
   * The list and map are two surfaces for one hover state. An old leave event
   * from one surface cannot clear a newer hover owned by the other.
   */
  const setHover = useCallback((id: string | null, origin: 'list' | 'map') => {
    const nextId = id !== null && publishedIdsRef.current.has(id) ? id : null;
    if (nextId === null && hoverOriginRef.current !== origin) return;

    const currentId = hoveredIdRef.current;
    if (currentId === nextId) {
      if (nextId !== null) hoverOriginRef.current = origin;
      return;
    }

    const map = mapRef.current;
    if (map?.getSource(SOURCE_ID)) {
      if (currentId !== null) setFeatureHover(map, currentId, false);
      if (nextId !== null) setFeatureHover(map, nextId, true);
    }

    hoveredIdRef.current = nextId;
    hoverOriginRef.current = nextId === null ? null : origin;
    setHoveredId(nextId);
  }, []);

  /** Fits either the published governorates or all 76 governorates. */
  const fitOverview = useCallback((map: MapLibreMap, mode: OverviewMode, animated: boolean) => {
    const bounds = boundsRef.current;
    if (!bounds) return;
    const [west, south, east, north] =
      mode === 'published' ? bounds.published : bounds.all;
    const padding = fitPadding(containerRef.current?.clientWidth ?? window.innerWidth);
    // The fitted overview also caps how far the camera may zoom out, so the
    // fully zoomed-out view settles on the same focus these buttons show, and
    // wheel or pinch zoom below that overview aims at the same focus.
    anchorBandRef.current = applyZoomOutFloor(map, bounds, mode, padding) ?? null;
    map.fitBounds(
      [
        [west, south],
        [east, north],
      ],
      {
        padding,
        // Keep MapLibre's camera center in the usable (padded) viewport. Without
        // absolute padding, fitBounds bakes the offset into a center over the
        // western blank area, so subsequent native zoom gestures target it.
        absolutePadding: true,
        duration: animated && !window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 700 : 0,
        essential: false,
      },
    );
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    let disposed = false;
    let resizeObserver: ResizeObserver | null = null;

    async function initialize() {
      try {
        const [collection, contextResponse] = await Promise.all([
          fetchCollection(controller.signal),
          fetch(HISTORICAL_CONTEXT_URL, { signal: controller.signal }),
        ]);
        if (!contextResponse.ok) {
          throw new Error(
            `Не удалось загрузить исторический контекст (HTTP ${contextResponse.status}).`,
          );
        }

        validateContext(await contextResponse.json());
        const container = containerRef.current;
        if (disposed || !container) return;

        applyCollection(collection);
        const bounds = boundsRef.current;
        if (!bounds) return;
        const { published: publishedBounds, all: allBounds } = bounds;

        const map = new maplibregl.Map({
          container,
          style: BASE_STYLE,
          center: [
            (publishedBounds[0] + publishedBounds[2]) / 2,
            (publishedBounds[1] + publishedBounds[3]) / 2,
          ],
          zoom: 3,
          minZoom: 0,
          maxZoom: 10,
          maxBounds: expandedMaxBounds(allBounds),
          attributionControl: false,
          dragRotate: false,
          pitchWithRotate: false,
          touchPitch: false,
        });
        mapRef.current = map;
        map.touchZoomRotate.disableRotation();
        map.addControl(new maplibregl.NavigationControl({ showCompass: false }), 'bottom-right');
        map.addControl(new maplibregl.ScaleControl({ unit: 'metric', maxWidth: 110 }), 'bottom-left');

        const handleMapError = (event: unknown) => {
          if (disposed) return;
          const error = (event as { error?: unknown } | null)?.error;
          setErrorMessage(error instanceof Error ? error.message : 'Неизвестная ошибка карты.');
          setStatus('error');
        };

        // One pointer for both tabs: settlement dots in the settlement tab,
        // published governorates in the gubernia tab.
        const updateCursor = () => {
          const overInteractive =
            viewRef.current === 'settlements'
              ? settlementHoverRef.current !== null
              : hoveredIdRef.current !== null;
          map.getCanvas().style.cursor = overInteractive ? 'pointer' : '';
        };

        const handleMouseMove = (event: MapLayerMouseEvent) => {
          if (viewRef.current !== 'gubernias') return;
          const feature = event.features?.[0];
          const rawId = feature?.id;
          if (feature?.properties?.published !== true || rawId === undefined || rawId === null)
            return;
          const id = String(rawId);
          if (!publishedIdsRef.current.has(id)) return;
          setHover(id, 'map');
          updateCursor();
        };

        const resetMapHover = () => {
          setHover(null, 'map');
          updateCursor();
        };

        const handleClick = (event: MapLayerMouseEvent) => {
          if (viewRef.current !== 'gubernias') return;
          const feature = event.features?.[0];
          const id = feature?.id;
          const slug = feature?.properties?.slug;
          if (
            feature?.properties?.published === true &&
            id !== undefined &&
            id !== null &&
            publishedIdsRef.current.has(String(id)) &&
            typeof slug === 'string' &&
            slug.length > 0
          ) {
            router.push(`/guberniya/${encodeURIComponent(slug)}`);
          }
        };

        const handleSettlementMove = (event: MapLayerMouseEvent) => {
          if (viewRef.current !== 'settlements') return;
          const rawId = event.features?.[0]?.id;
          if (rawId === undefined || rawId === null) return;
          setSettlementHover(String(rawId), 'map');
          updateCursor();
        };

        const clearSettlementHover = () => {
          setSettlementHover(null, 'map');
          updateCursor();
        };

        const handleSettlementClick = (event: MapLayerMouseEvent) => {
          if (viewRef.current !== 'settlements') return;
          // The store owns the local page address; the map only follows it.
          const url = event.features?.[0]?.properties?.url;
          if (typeof url === 'string' && url.length > 0) router.push(settlementHref(url));
        };

        const clearHovers = () => {
          resetMapHover();
          clearSettlementHover();
        };

        // Only neutral labels need a per-feature zoom threshold. In the
        // published overview show every published name, even if text overhangs.
        let visibleLabelBand = -1;
        let visibleLabelOverview: OverviewMode | null = null;
        let visibleLabelView: HomeView | null = null;
        const updateLabelFilter = () => {
          const band = Math.floor(map.getZoom() * 4) / 4;
          const overviewMode = overviewRef.current;
          const viewMode = viewRef.current;
          if (
            band === visibleLabelBand &&
            overviewMode === visibleLabelOverview &&
            viewMode === visibleLabelView
          ) {
            return;
          }
          visibleLabelBand = band;
          visibleLabelOverview = overviewMode;
          visibleLabelView = viewMode;
          if (map.getLayer(LABEL_LAYER_ID)) {
            map.setFilter(LABEL_LAYER_ID, [
              'all',
              ['==', ['get', 'published'], false],
              ['<=', ['get', 'labelMinZoom'], band],
            ]);
          }
          setLayerVisibility(
            map,
            PUBLISHED_LABEL_LAYER_ID,
            viewMode !== 'settlements' && (overviewMode === 'published' || band >= 3),
          );
          // Settlement names join once the empire overview is left behind.
          setLayerVisibility(
            map,
            SETTLEMENT_LABEL_LAYER_ID,
            viewMode === 'settlements' && band >= SETTLEMENT_LABEL_MIN_ZOOM,
          );
        };
        labelFilterRef.current = updateLabelFilter;

        // The settlement tab swaps the governorate presentation for the RiStat
        // reconstruction: 76 province fills with district outlines underneath,
        // published provinces beige. The governorate source and its labels step
        // aside, and the settlement points stay on top of both.
        const applyViewPresentation = () => {
          const settlementView = viewRef.current === 'settlements';
          setLayerVisibility(map, FILL_LAYER_ID, !settlementView);
          setLayerVisibility(map, BORDER_LAYER_ID, !settlementView);
          setLayerVisibility(map, PUBLISHED_FILL_LAYER_ID, !settlementView);
          setLayerVisibility(map, PUBLISHED_BORDER_LAYER_ID, !settlementView);
          setLayerVisibility(map, LABEL_LAYER_ID, !settlementView);
          setLayerVisibility(map, DISTRICT_PROVINCE_FILL_LAYER_ID, settlementView);
          setLayerVisibility(map, DISTRICT_BORDER_LAYER_ID, settlementView);
          setLayerVisibility(map, DISTRICT_PROVINCE_BORDER_LAYER_ID, settlementView);
          setLayerVisibility(
            map,
            PROVINCE_LABEL_LAYER_ID,
            settlementView && provinceLabelsRef.current !== null,
          );
          setLayerVisibility(map, SETTLEMENT_DOT_LAYER_ID, settlementView);
          updateLabelFilter();
        };
        viewSyncRef.current = applyViewPresentation;

        // While the camera is zoomed out past the current overview, wheel and pinch
        // zoom aim at that overview's focus rather than the pointer, so a zoom-in
        // leaving the fully zoomed-out state magnifies the published set instead of
        // the empty map under the cursor. The overview itself and every deeper
        // level keep MapLibre's pointer anchor. Evaluated per frame, since the
        // gesture handlers read their anchor again at each wheel step.
        let anchoredOnFocus = false;
        const updateZoomAnchor = () => {
          const band = anchorBandRef.current;
          const onFocus = band !== null && map.getZoom() < band - ZOOM_EPSILON;
          if (onFocus === anchoredOnFocus) return;
          anchoredOnFocus = onFocus;
          setFloorZoomAnchor(map, onFocus);
        };
        map.on('zoom', updateZoomAnchor);

        // Fully zoomed out, the camera must sit on the current overview focus:
        // MapLibre's max-bounds clamp alone parks it on the middle of the empire,
        // so a gesture started there magnified empty sea. A zoom-out settling on
        // the floor returns to that focus; pans and anything above the floor keep
        // the camera exactly where the user left it.
        let zoomAtGestureStart = map.getZoom();
        const recoverOverviewFocus = () => {
          const bounds = boundsRef.current;
          if (!bounds) return;
          const zoom = map.getZoom();
          const focus = overviewFocus(
            map,
            bounds,
            cameraOverview(),
            map.getPadding(),
          );
          if (!focus || focus.floor === undefined) return;
          if (zoom > focus.floor + ZOOM_EPSILON) return;
          // Only a zoom-out that bottomed out recovers; a pan or a zoom-in that
          // merely ended at the floor keeps the camera the user chose.
          if (zoom >= zoomAtGestureStart - ZOOM_EPSILON) return;
          const center = map.getCenter();
          if (
            Math.abs(center.lng - focus.center.lng) < FOCUS_EPSILON &&
            Math.abs(center.lat - focus.center.lat) < FOCUS_EPSILON
          ) {
            return;
          }
          map.jumpTo({ center: focus.center, zoom: focus.floor });
          settleZoomOutFloor(map, focus.floor);
        };
        map.on('zoomstart', () => {
          zoomAtGestureStart = map.getZoom();
        });
        map.on('zoomend', recoverOverviewFocus);

        map.once('load', () => {
          if (disposed) return;
          try {
            // Prefer the latest payload: a publication may have landed while
            // the style was still loading.
            const current = collectionRef.current ?? collection;
            addGuberniaSourceAndLayers(map, current, createLabelCollection(current));
            // Points come from the page payload and survive any data swap.
            (map.getSource(SETTLEMENT_SOURCE_ID) as GeoJSONSource | undefined)?.setData(
              settlementDataRef.current,
            );
            // A rebuilt style gets the already-loaded districts back; the first
            // entry into the settlement tab fills them in.
            const districtFeatures = districtFeaturesRef.current;
            if (districtFeatures) {
              (map.getSource(DISTRICT_SOURCE_ID) as GeoJSONSource | undefined)?.setData(
                withPublishedFlags(districtFeatures, publishedIdsRef.current),
              );
            }
            const provinceLabels = provinceLabelsRef.current;
            if (provinceLabels) {
              (map.getSource(PROVINCE_LABEL_SOURCE_ID) as GeoJSONSource | undefined)?.setData(
                provinceLabels,
              );
            }
            applyViewPresentation();
            fitOverview(map, cameraOverview(), false);
            map.on('zoom', updateLabelFilter);
            // Only the filtered published fill layer receives listeners.
            // Unpublished governorates cannot highlight, show a pointer, or navigate.
            map.on('mousemove', PUBLISHED_FILL_LAYER_ID, handleMouseMove);
            map.on('mouseleave', PUBLISHED_FILL_LAYER_ID, resetMapHover);
            map.on('click', PUBLISHED_FILL_LAYER_ID, handleClick);
            // Settlement points mirror those interactions one tab over.
            map.on('mousemove', SETTLEMENT_DOT_LAYER_ID, handleSettlementMove);
            map.on('mouseleave', SETTLEMENT_DOT_LAYER_ID, clearSettlementHover);
            map.on('click', SETTLEMENT_DOT_LAYER_ID, handleSettlementClick);
            const canvas = map.getCanvas();
            canvas.addEventListener('mouseleave', clearHovers);
            map.once('remove', () => canvas.removeEventListener('mouseleave', clearHovers));
            setStatus('ready');
          } catch (error) {
            handleMapError({ error });
          }
        });
        map.on('error', handleMapError);

        let currentBreakpoint = paddingBreakpoint(container.clientWidth);
        resizeObserver = new ResizeObserver(() => {
          map.resize();
          const nextBreakpoint = paddingBreakpoint(container.clientWidth);
          if (map.loaded() && nextBreakpoint !== currentBreakpoint) {
            currentBreakpoint = nextBreakpoint;
            fitOverview(map, cameraOverview(), false);
          }
        });
        resizeObserver.observe(container);
      } catch (error) {
        if (disposed || (error instanceof DOMException && error.name === 'AbortError')) return;
        setErrorMessage(error instanceof Error ? error.message : 'Не удалось подготовить карту.');
        setStatus('error');
      }
    }

    void initialize();

    return () => {
      disposed = true;
      controller.abort();
      districtRequestRef.current?.abort();
      districtRequestRef.current = null;
      resizeObserver?.disconnect();
      mapRef.current?.remove();
      mapRef.current = null;
      hoveredIdRef.current = null;
      hoverOriginRef.current = null;
      settlementHoverRef.current = null;
      settlementHoverOriginRef.current = null;
      labelFilterRef.current = null;
      viewSyncRef.current = null;
      collectionRef.current = null;
      publishedIdsRef.current = new Set();
      districtFeaturesRef.current = null;
      provinceLabelsRef.current = null;
    };
  }, [applyCollection, cameraOverview, fetchCollection, fitOverview, router, setHover, setSettlementHover]);

  // The native dialog element owns focus trapping, Escape, and backdrop
  // behaviour; React state stays the single source of its open flag.
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (isDialogOpen && !dialog.open) dialog.showModal();
    else if (!isDialogOpen && dialog.open) dialog.close();
  }, [isDialogOpen]);

  // The settlement editor gets its own dialog, so both admin forms stay
  // independent and neither nests a form inside another.
  useEffect(() => {
    const dialog = settlementDialogRef.current;
    if (!dialog) return;
    if (isSettlementDialogOpen && !dialog.open) dialog.showModal();
    else if (!isSettlementDialogOpen && dialog.open) dialog.close();
  }, [isSettlementDialogOpen]);

  function handleSettlementDialogClose() {
    setIsSettlementDialogOpen(false);
  }

  function handleSettlementDialogClick(event: ReactMouseEvent<HTMLDialogElement>) {
    // Show-modal dialogs deliver backdrop clicks to the dialog element itself.
    if (event.target === settlementDialogRef.current) setIsSettlementDialogOpen(false);
  }

  function openCreateDialog() {
    setSelectedId((current) =>
      unpublished.some((entry) => entry.id === current) ? current : (unpublished[0]?.id ?? ''),
    );
    setSlug('');
    setCreateError('');
    setCreateStatus('idle');
    setIsDialogOpen(true);
  }

  /** Covers Escape, the backdrop-less close event, and the cancel button. */
  function handleDialogClose() {
    setIsDialogOpen(false);
    setCreateStatus('idle');
    setCreateError('');
  }

  function handleDialogClick(event: ReactMouseEvent<HTMLDialogElement>) {
    // Show-modal dialogs deliver backdrop clicks to the dialog element itself.
    if (event.target === dialogRef.current) setIsDialogOpen(false);
  }

  async function handleCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (createStatus === 'submitting') return;
    const id = selectedId;
    const nextSlug = slug.trim();
    if (id.length === 0) {
      setCreateError('Выберите губернию из списка.');
      return;
    }
    if (!SLUG_PATTERN.test(nextSlug)) {
      setCreateError('Адрес страницы: строчные латинские буквы, цифры и дефисы.');
      return;
    }

    setCreateStatus('submitting');
    setCreateError('');
    let created = false;
    try {
      const response = await fetch(GUBERNIAS_URL, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id, slug: nextSlug }),
      });
      if (!response.ok) {
        const payload = (await response.json().catch(() => null)) as { error?: unknown } | null;
        throw new Error(
          typeof payload?.error === 'string' && payload.error.length > 0
            ? payload.error
            : `Не удалось опубликовать губернию (HTTP ${response.status}).`,
        );
      }

      // The merged collection is authoritative: refetch it so the list, the
      // count, and both map sources show the new governorate immediately.
      created = true;
      const updated = await fetchCollection();
      applyCollection(updated);
      setIsDialogOpen(false);
      setSlug('');

      // If the new governorate falls outside the current view, reveal it
      // without resetting the camera to the initial state.
      const map = mapRef.current;
      const feature = updated.features.find((candidate) => candidate.properties.id === id);
      if (map && feature) {
        const target = boundsForGeometry(feature.geometry);
        const visible = map.getBounds();
        const view: FeatureBounds = [
          visible.getWest(),
          visible.getSouth(),
          visible.getEast(),
          visible.getNorth(),
        ];
        const overlaps =
          target[0] <= view[2] &&
          target[2] >= view[0] &&
          target[1] <= view[3] &&
          target[3] >= view[1];
        if (!overlaps) fitOverview(map, cameraOverview(), true);
      }
    } catch (error) {
      if (created) {
        setCreateError(
          'Губерния опубликована, но обновить карту не удалось. Перезагрузите страницу.',
        );
      } else {
        setCreateError(
          error instanceof Error ? error.message : 'Не удалось опубликовать губернию.',
        );
      }
    } finally {
      setCreateStatus('idle');
    }
  }

  /**
   * The shared extent control flips the active tab's own overview between the
   * published governorates and the whole empire. Each tab keeps its choice.
   */
  function showOverview(next: OverviewMode) {
    const settlementView = viewRef.current === 'settlements';
    if (settlementView) {
      if (settlementOverviewRef.current === next) return;
      settlementOverviewRef.current = next;
      setSettlementOverview(next);
    } else {
      if (overviewRef.current === next) return;
      overviewRef.current = next;
      setOverview(next);
    }
    const map = mapRef.current;
    if (!map || !map.getSource(SOURCE_ID)) return;
    if (settlementView) {
      const settlementOrigin = settlementHoverOriginRef.current;
      if (settlementOrigin) setSettlementHover(null, settlementOrigin);
      map.getCanvas().style.cursor = '';
      fitOverview(map, next, true);
      return;
    }
    const origin = hoverOriginRef.current;
    if (origin) setHover(null, origin);
    map.getCanvas().style.cursor = '';
    labelFilterRef.current?.();
    fitOverview(map, next, true);
  }

  /**
   * Switches the homepage tab. Both tabs keep their own overview and the
   * shared extent control returns each one to the extent it last selected.
   */
  function showView(next: HomeView) {
    if (viewRef.current === next) return;
    viewRef.current = next;
    setView(next);
    if (next === 'settlements') void loadDistricts();
    const map = mapRef.current;
    if (!map || !map.getSource(SOURCE_ID)) return;
    const origin = hoverOriginRef.current;
    if (origin) setHover(null, origin);
    const settlementOrigin = settlementHoverOriginRef.current;
    if (settlementOrigin) setSettlementHover(null, settlementOrigin);
    map.getCanvas().style.cursor = '';
    viewSyncRef.current?.();
    fitOverview(map, cameraOverview(), true);
  }

  /** The extent the active tab's button reflects and toggles. */
  const activeOverview = view === 'settlements' ? settlementOverview : overview;

  /** Announced once status settles so slow loads never read "0 губерний". */
  const publishedSummary =
    status === 'ready'
      ? view === 'settlements'
        ? settlements.length === 0
          ? 'Населённые пункты пока не добавлены.'
          : `Показано ${settlementCountLabel(settlements.length)}.`
        : overview === 'empire'
          ? `Показаны все ${EXPECTED_FEATURE_COUNT} губерний 1897 года.`
          : publishedCountLabel(entries.length)
      : '';

  return (
    <section
      className={`${styles.shell} ${view === 'settlements' ? styles.settlementView : ''}`}
      aria-label={
        view === 'settlements' ? 'Карта населённых пунктов' : 'Интерактивная карта губерний'
      }
    >
      <div className={styles.viewport}>
        <div ref={containerRef} className={styles.map} />

        <div className={styles.periodCaption}>
          <span>Российская империя</span>
          <strong>
            {view === 'settlements' ? 'Населенные пункты · 1897' : 'Губернии · 1897'}
          </strong>
        </div>

        {/* The two homepage tabs; each keeps its own map presentation. */}
        <div className={styles.viewSwitch} role="group" aria-label="Слой карты">
          <button
            className={styles.viewSwitchButton}
            type="button"
            aria-pressed={view === 'gubernias'}
            disabled={status !== 'ready'}
            onClick={() => showView('gubernias')}
          >
            Губернии
          </button>
          <button
            className={styles.viewSwitchButton}
            type="button"
            aria-pressed={view === 'settlements'}
            disabled={status !== 'ready'}
            onClick={() => showView('settlements')}
          >
            Населенные пункты
          </button>
        </div>

        <aside className={styles.guberniasPanel} aria-labelledby="available-gubernias-title">
          <div className={styles.panelHeading}>
            <h2 id="available-gubernias-title">Доступные губернии</h2>
          </div>
          <ul className={styles.guberniasList} aria-busy={status === 'loading'}>
            {entries.map((entry) => (
              <li key={entry.id}>
                <Link
                  className={`${styles.guberniaLink} ${
                    hoveredId === entry.id ? styles.guberniaLinkHovered : ''
                  }`}
                  href={`/guberniya/${encodeURIComponent(entry.slug)}`}
                  onMouseEnter={() => setHover(entry.id, 'list')}
                  onMouseLeave={(event) => {
                    if (event.currentTarget !== document.activeElement) setHover(null, 'list');
                  }}
                  onFocus={() => setHover(entry.id, 'list')}
                  onBlur={(event) => {
                    if (!event.currentTarget.contains(event.relatedTarget)) setHover(null, 'list');
                  }}
                >
                  {entry.name}
                </Link>
              </li>
            ))}
          </ul>
          {isAdmin ? (
            <div className={styles.addRow}>
              <button
                className={styles.addButton}
                type="button"
                disabled={status === 'loading' || unpublished.length === 0}
                onClick={openCreateDialog}
              >
                + Добавить губернию
              </button>
            </div>
          ) : null}
        </aside>

        <div className={styles.extentPanel}>
          <button
            className={styles.extentButton}
            type="button"
            disabled={status !== 'ready'}
            aria-pressed={activeOverview === 'empire'}
            onClick={() => showOverview(activeOverview === 'published' ? 'empire' : 'published')}
          >
            <span className={styles.extentTitle}>
              {activeOverview === 'published' ? 'Вся империя' : publishedCountLabel(entries.length)}
            </span>
            <span className={styles.extentHint}>
              {activeOverview === 'published'
                ? `${EXPECTED_FEATURE_COUNT} губерний · 1897`
                : 'вернуться к обзору'}
            </span>
          </button>
        </div>

        {view === 'settlements' ? (
          <SettlementListPanel
            settlements={settlements}
            provinces={provinces}
            hoveredId={settlementHoveredId}
            onHover={hoverSettlementFromList}
            onAdd={isAdmin ? () => setIsSettlementDialogOpen(true) : undefined}
          />
        ) : null}

        {view === 'settlements' && districtStatus === 'loading' ? (
          <div className={styles.statePanel} role="status">
            <span className={styles.loader} aria-hidden="true" />
            Загружаем уезды 1897 года…
          </div>
        ) : null}

        {view === 'settlements' && districtStatus === 'error' ? (
          <div className={styles.errorPanel} role="alert">
            <strong>Уезды недоступны</strong>
            <span>{districtError}</span>
            <button
              className={styles.retryButton}
              type="button"
              onClick={() => void loadDistricts()}
            >
              Попробовать снова
            </button>
          </div>
        ) : null}

        {status === 'loading' ? (
          <div className={styles.statePanel} role="status">
            <span className={styles.loader} aria-hidden="true" />
            Загружаем схему…
          </div>
        ) : null}

        {status === 'error' ? (
          <div className={styles.errorPanel} role="alert">
            <strong>Карта недоступна</strong>
            <span>{errorMessage}</span>
          </div>
        ) : null}
      </div>

      <span className={styles.visuallyHidden} role="status">
        {publishedSummary}
      </span>

      {isAdmin ? (
        <dialog
          ref={dialogRef}
          className={styles.createDialog}
          aria-labelledby="create-gubernia-title"
          onClose={handleDialogClose}
          onClick={handleDialogClick}
        >
          <form className={styles.createForm} onSubmit={handleCreate}>
            <h2 id="create-gubernia-title">Добавить губернию</h2>
            <p className={styles.createHint}>
              Губерния станет доступной в списке и на карте сразу после публикации.
            </p>
            <label className={styles.createField}>
              <span>Название губернии</span>
              <select
                value={selectedId}
                onChange={(event) => setSelectedId(event.target.value)}
                disabled={unpublished.length === 0}
                required
              >
                {unpublished.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.name}
                  </option>
                ))}
              </select>
            </label>
            <label className={styles.createField}>
              <span>Slug (адрес страницы)</span>
              <input
                value={slug}
                onChange={(event) => setSlug(event.target.value)}
                placeholder="например, kazan"
                maxLength={100}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                required
              />
            </label>
            {createError.length > 0 ? (
              <p className={styles.createError} role="alert">
                {createError}
              </p>
            ) : null}
            <div className={styles.createActions}>
              <button
                className={styles.createCancel}
                type="button"
                onClick={() => setIsDialogOpen(false)}
              >
                Отмена
              </button>
              <button
                className={styles.createSubmit}
                type="submit"
                disabled={createStatus === 'submitting' || unpublished.length === 0}
              >
                {createStatus === 'submitting' ? 'Создаём…' : 'Создать'}
              </button>
            </div>
          </form>
        </dialog>
      ) : null}

      {isAdmin ? (
        <dialog
          ref={settlementDialogRef}
          className={styles.settlementDialog}
          aria-label="Добавить населённый пункт"
          onClose={handleSettlementDialogClose}
          onClick={handleSettlementDialogClick}
        >
          {isSettlementDialogOpen ? (
            // The editor keeps its own form, so the dialog adds no second one;
            // the body provides the padding the compact editor drops.
            <div className={styles.settlementDialogBody}>
              <SettlementEditor
                guberniaId=""
                provinces={provinces}
                settlementTypes={settings.settlementTypes}
                compact
                onDone={handleSettlementDialogClose}
              />
            </div>
          ) : null}
        </dialog>
      ) : null}
    </section>
  );
}
