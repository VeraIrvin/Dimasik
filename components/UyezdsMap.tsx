'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import * as maplibregl from 'maplibre-gl';
import type {
  GeoJSONSource,
  LngLatBoundsLike,
  Map as MapLibreMap,
  MapLayerMouseEvent,
  StyleSpecification,
} from 'maplibre-gl';
import type { Feature, FeatureCollection, MultiPolygon, Point, Polygon } from 'geojson';
import type { Settlement } from '../lib/gubernia-publications';
import { districtLabelCollection } from '../lib/district-labels';
import styles from './UyezdsMap.module.css';

// Keep the worker version and local serving convention aligned with the main map.
maplibregl.setWorkerUrl('/maplibre/maplibre-gl-worker.mjs');

const SOURCE_ID = 'uyezds-1897';
const DISTRICT_LABEL_SOURCE_ID = 'uyezds-district-labels';
const PROVINCE_FILL_LAYER_ID = 'uyezds-province-fill';
const PROVINCE_OUTLINE_LAYER_ID = 'uyezds-province-outline';
const DISTRICT_FILL_LAYER_ID = 'uyezds-district-fill';
const DISTRICT_BORDER_LAYER_ID = 'uyezds-district-border';
const DISTRICT_LABEL_LAYER_ID = 'uyezds-district-label';
const SETTLEMENT_SOURCE_ID = 'settlements';
const SETTLEMENT_DOT_LAYER_ID = 'settlement-dots';
const SETTLEMENT_LABEL_LAYER_ID = 'settlement-labels';
// Settlement markers stay hidden while the province overview is on screen: below this
// zoom province-wide markers overlap into an unreadable band.
const SETTLEMENT_READABLE_MIN_ZOOM = 7;
// On top of that, reveal only once the user zooms a full step past the province overview
// they are reading, because small provinces already fit the viewport above the readable zoom.
const SETTLEMENT_OVERVIEW_REVEAL_MARGIN = 1;

const BASE_STYLE: StyleSpecification = {
  version: 8,
  sources: {},
  layers: [{ id: 'canvas', type: 'background', paint: { 'background-color': '#f7f6f2' } }],
};

const DISTRICT_LABEL_TEXT_SIZE: ['interpolate', ['linear'], ['zoom'], number, number, number, number] =
  ['interpolate', ['linear'], ['zoom'], 5, 10, 8, 12];

type Bounds = [number, number, number, number];
type LoadStatus = 'loading' | 'ready' | 'error';
type HoverOrigin = 'list' | 'map';
type ProvinceProperties = { kind: 'province'; id: string; name: string };
type DistrictProperties = { kind: 'district'; id: string; name: string; provinceId: string };
type ProvinceFeature = Feature<Polygon | MultiPolygon, ProvinceProperties>;
type DistrictFeature = Feature<Polygon | MultiPolygon, DistrictProperties>;
type UyezdsCollection = FeatureCollection<
  Polygon | MultiPolygon,
  ProvinceProperties | DistrictProperties
>;
type DistrictEntry = { id: string; name: string; bounds: Bounds };
type SettlementProperties = { id: string; name: string; uyezdId: string };
type SettlementCollection = FeatureCollection<Point, SettlementProperties>;

type UyezdsMapProps = {
  guberniaId: string;
  guberniaName: string;
  settlements: Settlement[];
  selectedDistrictId: string | null;
  selectedSettlementId: string | null;
  onDistrictSelect: (id: string | null) => void;
  onSettlementOpen: (id: string) => void;
  onDistrictsLoad?: (districts: { id: string; name: string }[]) => void;
};

function geometryBounds(geometry: Polygon | MultiPolygon, label: string): Bounds {
  let west = Number.POSITIVE_INFINITY;
  let south = Number.POSITIVE_INFINITY;
  let east = Number.NEGATIVE_INFINITY;
  let north = Number.NEGATIVE_INFINITY;
  let positions = 0;

  const visit = (value: unknown): void => {
    if (!Array.isArray(value)) throw new Error(`${label}: координаты имеют неверный формат.`);
    if (value.length >= 2 && typeof value[0] === 'number' && typeof value[1] === 'number') {
      const longitude = value[0];
      const latitude = value[1];
      if (
        !Number.isFinite(longitude) ||
        !Number.isFinite(latitude) ||
        longitude < -180 ||
        longitude > 180 ||
        latitude < -90 ||
        latitude > 90
      ) {
        throw new Error(`${label}: найдены недопустимые координаты.`);
      }
      positions += 1;
      west = Math.min(west, longitude);
      south = Math.min(south, latitude);
      east = Math.max(east, longitude);
      north = Math.max(north, latitude);
      return;
    }
    value.forEach(visit);
  };

  visit(geometry.coordinates);
  if (positions < 4) throw new Error(`${label}: геометрия не содержит полноценного контура.`);
  return [west, south, east, north];
}

function parseCollection(
  raw: unknown,
  guberniaId: string,
  guberniaName: string,
): { collection: UyezdsCollection; province: ProvinceFeature; districts: DistrictEntry[] } {
  const candidate = raw as { type?: unknown; features?: unknown } | null;
  if (
    candidate === null ||
    typeof candidate !== 'object' ||
    candidate.type !== 'FeatureCollection' ||
    !Array.isArray(candidate.features)
  ) {
    throw new Error('Файл уездов имеет неверный формат GeoJSON.');
  }
  if (candidate.features.length < 2) {
    throw new Error('В файле нет границ губернии и её уездов.');
  }

  const ids = new Set<string>();
  const parsed = candidate.features.map((rawFeature, index) => {
    const feature = rawFeature as {
      type?: unknown;
      properties?: Record<string, unknown> | null;
      geometry?: { type?: unknown; coordinates?: unknown } | null;
    } | null;
    const properties = feature?.properties;
    const geometry = feature?.geometry;
    const expectedKind = index === 0 ? 'province' : 'district';

    if (
      feature?.type !== 'Feature' ||
      properties === null ||
      typeof properties !== 'object' ||
      properties.kind !== expectedKind ||
      typeof properties.id !== 'string' ||
      properties.id.trim().length === 0 ||
      typeof properties.name !== 'string' ||
      properties.name.trim().length === 0 ||
      geometry === null ||
      typeof geometry !== 'object' ||
      (geometry.type !== 'Polygon' && geometry.type !== 'MultiPolygon') ||
      !Array.isArray(geometry.coordinates)
    ) {
      throw new Error(`Объект №${index + 1} не соответствует схеме границ 1897 года.`);
    }
    if (ids.has(properties.id)) {
      throw new Error(`Идентификатор «${properties.id}» повторяется.`);
    }
    ids.add(properties.id);

    if (expectedKind === 'province') {
      if (properties.id !== guberniaId || properties.name.trim() !== guberniaName.trim()) {
        throw new Error('Граница губернии не соответствует открытой странице.');
      }
    } else if (properties.provinceId !== guberniaId) {
      throw new Error(`Уезд «${properties.name}» относится к другой губернии.`);
    }

    const parsedGeometry = {
      type: geometry.type,
      coordinates: geometry.coordinates,
    } as Polygon | MultiPolygon;
    geometryBounds(parsedGeometry, `Объект «${properties.name}»`);

    return {
      type: 'Feature',
      properties:
        expectedKind === 'province'
          ? { kind: 'province', id: properties.id, name: properties.name }
          : {
              kind: 'district',
              id: properties.id,
              name: properties.name,
              provinceId: properties.provinceId as string,
            },
      geometry: parsedGeometry,
    } as ProvinceFeature | DistrictFeature;
  });

  const province = parsed[0] as ProvinceFeature;
  const districtFeatures = parsed.slice(1) as DistrictFeature[];
  const districts = districtFeatures
    .map((feature) => ({
      id: feature.properties.id,
      name: feature.properties.name,
      bounds: geometryBounds(feature.geometry, `Уезд «${feature.properties.name}»`),
    }))
    .sort((left, right) => left.name.localeCompare(right.name, 'ru'));

  return {
    collection: { type: 'FeatureCollection', features: parsed },
    province,
    districts,
  };
}

function expandedMaxBounds([west, south, east, north]: Bounds): LngLatBoundsLike {
  // Give fitBounds enough room to zoom out on tall, narrow provinces even when
  // the floating list consumes much of a short viewport.
  const horizontal = Math.max((east - west) * 1.8, 1.5);
  const vertical = Math.max((north - south) * 1.8, 1.5);
  return [
    [Math.max(-180, west - horizontal), Math.max(-85, south - vertical)],
    [Math.min(180, east + horizontal), Math.min(85, north + vertical)],
  ];
}

function fitPadding(width: number) {
  if (width <= 600) return { top: 152, right: 16, bottom: 24, left: 16 };
  if (width <= 900) return { top: 36, right: 36, bottom: 44, left: 254 };
  return { top: 44, right: 44, bottom: 52, left: 286 };
}

// Zoom at which settlement markers become readable for the province on screen. A province
// fits the viewport at a different zoom depending on its size and on the viewport, so this
// mirrors the zoom `fitBounds(provinceBounds)` reaches instead of assuming one number fits
// every province, and demands a clear zoom step past that overview.
function settlementRevealZoom(map: MapLibreMap, provinceBounds: Bounds | null): number {
  const maxZoom = map.getMaxZoom();
  const readableZoom = Math.min(SETTLEMENT_READABLE_MIN_ZOOM, maxZoom);
  if (provinceBounds === null) return readableZoom;
  const overviewCamera = map.cameraForBounds(
    [
      [provinceBounds[0], provinceBounds[1]],
      [provinceBounds[2], provinceBounds[3]],
    ],
    { padding: fitPadding(map.getContainer().clientWidth) },
  );
  const overviewZoom = overviewCamera?.zoom;
  if (overviewZoom === undefined) return readableZoom;
  return Math.min(
    Math.max(readableZoom, overviewZoom + SETTLEMENT_OVERVIEW_REVEAL_MARGIN),
    maxZoom,
  );
}

function setFeatureState(
  map: MapLibreMap,
  source: string,
  id: string,
  state: 'hover' | 'selected',
  value: boolean,
) {
  if (!map.getSource(source)) return;
  map.setFeatureState({ source, id }, { [state]: value });
}

function settlementCollection(settlements: Settlement[]): SettlementCollection {
  return {
    type: 'FeatureCollection',
    features: settlements.map((settlement) => ({
      type: 'Feature',
      id: settlement.id,
      properties: {
        id: settlement.id,
        name: settlement.name,
        uyezdId: settlement.uyezdId,
      },
      geometry: {
        type: 'Point',
        coordinates: [settlement.longitude, settlement.latitude],
      },
    })),
  };
}

function addSourceAndLayers(map: MapLibreMap, collection: UyezdsCollection) {
  map.addSource(SOURCE_ID, { type: 'geojson', data: collection, promoteId: 'id' });
  // Polygon labels are placed once per rendered tile; one point per district
  // prevents the same name from recurring as the user zooms in.
  map.addSource(DISTRICT_LABEL_SOURCE_ID, {
    type: 'geojson',
    data: districtLabelCollection(collection.features.slice(1) as DistrictFeature[]),
  });
  map.addLayer({
    id: PROVINCE_FILL_LAYER_ID,
    type: 'fill',
    source: SOURCE_ID,
    filter: ['==', ['get', 'kind'], 'province'],
    paint: { 'fill-color': '#eee9dd', 'fill-opacity': 0.92 },
  });
  map.addLayer({
    id: DISTRICT_FILL_LAYER_ID,
    type: 'fill',
    source: SOURCE_ID,
    filter: ['==', ['get', 'kind'], 'district'],
    paint: {
      'fill-color': [
        'case',
        ['boolean', ['feature-state', 'selected'], false],
        '#d4bd86',
        ['boolean', ['feature-state', 'hover'], false],
        '#e1cfa5',
        '#e9dfc8',
      ],
      'fill-opacity': 0.86,
      'fill-color-transition': { duration: 140, delay: 0 },
    },
  });
  map.addLayer({
    id: DISTRICT_BORDER_LAYER_ID,
    type: 'line',
    source: SOURCE_ID,
    filter: ['==', ['get', 'kind'], 'district'],
    paint: {
      'line-color': [
        'case',
        ['boolean', ['feature-state', 'selected'], false],
        '#685223',
        ['boolean', ['feature-state', 'hover'], false],
        '#806a38',
        '#aaa18f',
      ],
      'line-width': [
        'case',
        ['boolean', ['feature-state', 'selected'], false],
        2.4,
        ['boolean', ['feature-state', 'hover'], false],
        1.8,
        0.9,
      ],
      'line-opacity': 0.96,
      'line-color-transition': { duration: 140, delay: 0 },
      'line-width-transition': { duration: 140, delay: 0 },
    },
  });
  map.addLayer({
    id: PROVINCE_OUTLINE_LAYER_ID,
    type: 'line',
    source: SOURCE_ID,
    filter: ['==', ['get', 'kind'], 'province'],
    paint: { 'line-color': '#665f52', 'line-width': 2.2, 'line-opacity': 0.95 },
  });
  map.addLayer({
    id: DISTRICT_LABEL_LAYER_ID,
    type: 'symbol',
    source: DISTRICT_LABEL_SOURCE_ID,
    layout: {
      'symbol-placement': 'point',
      'text-field': ['get', 'name'],
      'text-font': ['Arial', 'Helvetica', 'sans-serif'],
      'text-size': DISTRICT_LABEL_TEXT_SIZE,
      'text-max-width': 11,
      'text-padding': 3,
      'text-anchor': 'center',
      'text-justify': 'center',
      'text-allow-overlap': false,
      'text-ignore-placement': false,
    },
    paint: {
      'text-color': '#4b463d',
      'text-halo-color': '#f7f6f2',
      'text-halo-width': 1.2,
      'text-halo-blur': 0.2,
    },
  });
  map.addSource(SETTLEMENT_SOURCE_ID, {
    type: 'geojson',
    data: settlementCollection([]),
    promoteId: 'id',
  });
  map.addLayer({
    id: SETTLEMENT_DOT_LAYER_ID,
    type: 'circle',
    source: SETTLEMENT_SOURCE_ID,
    layout: {
      visibility: 'none',
    },
    paint: {
      'circle-radius': [
        'case',
        ['boolean', ['feature-state', 'selected'], false],
        6.5,
        ['boolean', ['feature-state', 'hover'], false],
        5.5,
        4.2,
      ],
      'circle-color': [
        'case',
        ['boolean', ['feature-state', 'selected'], false],
        '#685223',
        ['boolean', ['feature-state', 'hover'], false],
        '#8a6d2f',
        '#a08a51',
      ],
      'circle-stroke-color': '#fffdf7',
      'circle-stroke-width': [
        'case',
        ['boolean', ['feature-state', 'selected'], false],
        2,
        1.4,
      ],
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
      'symbol-placement': 'point',
      'text-field': ['get', 'name'],
      'text-font': ['Arial', 'Helvetica', 'sans-serif'],
      'text-size': ['interpolate', ['linear'], ['zoom'], 5, 10, 8, 12],
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

export default function UyezdsMap({
  guberniaId,
  guberniaName,
  settlements,
  selectedDistrictId,
  selectedSettlementId,
  onDistrictSelect,
  onSettlementOpen,
  onDistrictsLoad,
}: UyezdsMapProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<MapLibreMap | null>(null);
  const provinceBoundsRef = useRef<Bounds | null>(null);
  const districtBoundsRef = useRef<Map<string, Bounds>>(new Map());
  const districtIdsRef = useRef<Set<string>>(new Set());
  const settlementIdsRef = useRef<Set<string>>(new Set());
  const hoveredIdRef = useRef<string | null>(null);
  const hoverOriginRef = useRef<HoverOrigin | null>(null);
  const hoveredSettlementIdRef = useRef<string | null>(null);
  const appliedViewRef = useRef<string | null>(null);
  const settlementLayersVisibleRef = useRef<boolean | null>(null);

  const [districts, setDistricts] = useState<DistrictEntry[]>([]);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [status, setStatus] = useState<LoadStatus>('loading');
  const [errorMessage, setErrorMessage] = useState('');

  // The map instance outlives single renders, so its long-lived event handlers
  // read the freshest props and callbacks through this ref.
  const latestRef = useRef({
    onDistrictSelect,
    onSettlementOpen,
    onDistrictsLoad,
    settlements,
    selectedDistrictId,
    selectedSettlementId,
  });

  useEffect(() => {
    latestRef.current = {
      onDistrictSelect,
      onSettlementOpen,
      onDistrictsLoad,
      settlements,
      selectedDistrictId,
      selectedSettlementId,
    };
  });

  const settlementData = useMemo(() => settlementCollection(settlements), [settlements]);

  const fitBounds = useCallback((bounds: Bounds, animated: boolean, maxZoom?: number) => {
    const map = mapRef.current;
    if (!map) return;
    map.fitBounds(
      [
        [bounds[0], bounds[1]],
        [bounds[2], bounds[3]],
      ],
      {
        padding: fitPadding(containerRef.current?.clientWidth ?? window.innerWidth),
        duration: animated && !window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 600 : 0,
        essential: false,
        ...(maxZoom === undefined ? {} : { maxZoom }),
      },
    );
  }, []);

  const setHover = useCallback((id: string | null, origin: HoverOrigin) => {
    const nextId = id !== null && districtIdsRef.current.has(id) ? id : null;
    if (nextId === null && hoverOriginRef.current !== origin) return;
    const currentId = hoveredIdRef.current;
    if (currentId === nextId) {
      if (nextId !== null) hoverOriginRef.current = origin;
      return;
    }
    const map = mapRef.current;
    if (map) {
      if (currentId !== null) setFeatureState(map, SOURCE_ID, currentId, 'hover', false);
      if (nextId !== null) setFeatureState(map, SOURCE_ID, nextId, 'hover', true);
    }
    hoveredIdRef.current = nextId;
    hoverOriginRef.current = nextId === null ? null : origin;
    setHoveredId(nextId);
  }, []);

  const setSettlementHover = useCallback((id: string | null) => {
    const nextId = id !== null && settlementIdsRef.current.has(id) ? id : null;
    const currentId = hoveredSettlementIdRef.current;
    if (currentId === nextId) return;
    const map = mapRef.current;
    if (map) {
      if (currentId !== null) setFeatureState(map, SETTLEMENT_SOURCE_ID, currentId, 'hover', false);
      if (nextId !== null) setFeatureState(map, SETTLEMENT_SOURCE_ID, nextId, 'hover', true);
    }
    hoveredSettlementIdRef.current = nextId;
  }, []);

  const syncSettlementVisibility = useCallback(() => {
    const map = mapRef.current;
    if (
      !map ||
      !map.getLayer(SETTLEMENT_DOT_LAYER_ID) ||
      !map.getLayer(SETTLEMENT_LABEL_LAYER_ID)
    ) {
      return;
    }
    const visible =
      latestRef.current.selectedDistrictId !== null ||
      latestRef.current.selectedSettlementId !== null ||
      map.getZoom() >= settlementRevealZoom(map, provinceBoundsRef.current);
    if (settlementLayersVisibleRef.current === visible) return;

    const visibility = visible ? 'visible' : 'none';
    map.setLayoutProperty(SETTLEMENT_DOT_LAYER_ID, 'visibility', visibility);
    map.setLayoutProperty(SETTLEMENT_LABEL_LAYER_ID, 'visibility', visibility);
    settlementLayersVisibleRef.current = visible;

    if (!visible) {
      setSettlementHover(null);
      map.getCanvas().style.cursor = hoveredIdRef.current === null ? '' : 'pointer';
    }
  }, [setSettlementHover]);

  // Controlled selection: selection and hover props repaint feature states.
  const syncFeatureStates = useCallback(() => {
    const map = mapRef.current;
    if (!map) return;
    const districtId = latestRef.current.selectedDistrictId;
    const hoveredDistrictId = hoveredIdRef.current;
    for (const id of districtIdsRef.current) {
      setFeatureState(map, SOURCE_ID, id, 'selected', id === districtId);
      setFeatureState(map, SOURCE_ID, id, 'hover', id === hoveredDistrictId);
    }
    const settlementId = latestRef.current.selectedSettlementId;
    const hoveredSettlementId = hoveredSettlementIdRef.current;
    for (const id of settlementIdsRef.current) {
      setFeatureState(map, SETTLEMENT_SOURCE_ID, id, 'selected', id === settlementId);
      setFeatureState(map, SETTLEMENT_SOURCE_ID, id, 'hover', id === hoveredSettlementId);
    }
  }, []);

  const focusSettlement = useCallback((settlement: Settlement) => {
    const map = mapRef.current;
    if (!map) return;
    map.flyTo({
      center: [settlement.longitude, settlement.latitude],
      zoom: Math.max(map.getZoom(), 9.5),
      duration: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 600,
      essential: false,
    });
  }, []);

  useEffect(() => {
    if (status !== 'ready') return;
    syncFeatureStates();
    syncSettlementVisibility();
    const settlement =
      selectedSettlementId === null
        ? null
        : (settlements.find((entry) => entry.id === selectedSettlementId) ?? null);
    const viewKey =
      settlement !== null
        ? `settlement:${settlement.id}:${settlement.longitude},${settlement.latitude}`
        : selectedDistrictId !== null && districtBoundsRef.current.has(selectedDistrictId)
          ? `district:${selectedDistrictId}`
          : 'province';
    if (appliedViewRef.current === viewKey) return;
    appliedViewRef.current = viewKey;
    if (settlement !== null) {
      focusSettlement(settlement);
      return;
    }
    if (viewKey.startsWith('district:')) {
      const bounds = districtBoundsRef.current.get(selectedDistrictId as string);
      if (bounds) fitBounds(bounds, true, 9);
      return;
    }
    if (provinceBoundsRef.current) fitBounds(provinceBoundsRef.current, true);
  }, [
    fitBounds,
    focusSettlement,
    selectedDistrictId,
    selectedSettlementId,
    settlements,
    status,
    syncFeatureStates,
    syncSettlementVisibility,
  ]);

  // Markers follow the `settlements` prop, so points created elsewhere show up
  // after router.refresh() without recreating the map.
  useEffect(() => {
    if (status !== 'ready') return;
    const map = mapRef.current;
    const source = map?.getSource(SETTLEMENT_SOURCE_ID) as GeoJSONSource | undefined;
    if (!map || !source) return;
    source.setData(settlementData);
    settlementIdsRef.current = new Set(settlements.map((entry) => entry.id));
    if (
      hoveredSettlementIdRef.current !== null &&
      !settlementIdsRef.current.has(hoveredSettlementIdRef.current)
    ) {
      hoveredSettlementIdRef.current = null;
    }
    syncFeatureStates();
  }, [settlementData, settlements, status, syncFeatureStates]);

  useEffect(() => {
    const controller = new AbortController();
    let disposed = false;
    let resizeObserver: ResizeObserver | null = null;

    setStatus('loading');
    setErrorMessage('');
    setDistricts([]);
    setHoveredId(null);
    hoveredIdRef.current = null;
    hoverOriginRef.current = null;
    hoveredSettlementIdRef.current = null;
    appliedViewRef.current = null;
    provinceBoundsRef.current = null;
    districtBoundsRef.current = new Map();
    districtIdsRef.current = new Set();
    settlementIdsRef.current = new Set();
    settlementLayersVisibleRef.current = null;

    async function initialize() {
      try {
        const response = await fetch(
          `/data/uyezds-1897/${encodeURIComponent(guberniaId)}.geojson`,
          { signal: controller.signal },
        );
        if (!response.ok) {
          throw new Error(`Не удалось загрузить границы уездов (HTTP ${response.status}).`);
        }
        const parsed = parseCollection(await response.json(), guberniaId, guberniaName);
        const container = containerRef.current;
        if (disposed || !container) return;

        const provinceBounds = geometryBounds(parsed.province.geometry, 'Граница губернии');
        provinceBoundsRef.current = provinceBounds;
        districtBoundsRef.current = new Map(
          parsed.districts.map((district) => [district.id, district.bounds]),
        );
        districtIdsRef.current = new Set(parsed.districts.map((district) => district.id));
        setDistricts(parsed.districts);
        latestRef.current.onDistrictsLoad?.(
          parsed.districts.map((district) => ({ id: district.id, name: district.name })),
        );

        const map = new maplibregl.Map({
          container,
          style: BASE_STYLE,
          center: [
            (provinceBounds[0] + provinceBounds[2]) / 2,
            (provinceBounds[1] + provinceBounds[3]) / 2,
          ],
          zoom: 5,
          minZoom: 2,
          maxZoom: 11,
          maxBounds: expandedMaxBounds(provinceBounds),
          attributionControl: false,
          dragRotate: false,
          pitchWithRotate: false,
          touchPitch: false,
        });
        mapRef.current = map;
        map.touchZoomRotate.disableRotation();
        map.addControl(new maplibregl.NavigationControl({ showCompass: false }), 'bottom-right');

        const canvas = map.getCanvas();
        canvas.setAttribute('aria-label', `Карта уездов: ${guberniaName}, 1897 год`);

        const reportMapError = (event: unknown) => {
          if (disposed) return;
          const error = (event as { error?: unknown } | null)?.error;
          setErrorMessage(error instanceof Error ? error.message : 'Неизвестная ошибка карты.');
          setStatus('error');
        };
        const updateCursor = () => {
          canvas.style.cursor =
            hoveredIdRef.current !== null || hoveredSettlementIdRef.current !== null
              ? 'pointer'
              : '';
        };
        const settlementIdAt = (event: MapLayerMouseEvent) => {
          const feature = event.features?.[0];
          const rawId = feature?.properties?.id ?? feature?.id;
          if (rawId === undefined || rawId === null) return null;
          const id = String(rawId);
          return settlementIdsRef.current.has(id) ? id : null;
        };
        const handleSettlementMove = (event: MapLayerMouseEvent) => {
          const id = settlementIdAt(event);
          if (id === null) return;
          setSettlementHover(id);
          updateCursor();
        };
        const clearSettlementHover = () => {
          setSettlementHover(null);
          updateCursor();
        };
        const handleSettlementClick = (event: MapLayerMouseEvent) => {
          const id = settlementIdAt(event);
          if (id === null) return;
          event.preventDefault();
          latestRef.current.onSettlementOpen(id);
        };
        const handleMapMove = (event: MapLayerMouseEvent) => {
          const rawId = event.features?.[0]?.id;
          if (rawId === undefined || rawId === null) return;
          const id = String(rawId);
          if (!districtIdsRef.current.has(id)) return;
          setHover(id, 'map');
          updateCursor();
        };
        const clearMapHover = () => {
          setHover(null, 'map');
          updateCursor();
        };
        const handleMapClick = (event: MapLayerMouseEvent) => {
          // A marker lies inside a polygon, so marker and label clicks must not select its district.
          const marks = map.queryRenderedFeatures(event.point, {
            layers: [SETTLEMENT_DOT_LAYER_ID, SETTLEMENT_LABEL_LAYER_ID],
          });
          if (marks.length > 0) return;
          const rawId = event.features?.[0]?.id;
          if (rawId === undefined || rawId === null) return;
          const id = String(rawId);
          if (districtIdsRef.current.has(id)) latestRef.current.onDistrictSelect(id);
        };
        const clearHovers = () => {
          setHover(null, 'map');
          setSettlementHover(null);
          canvas.style.cursor = '';
        };

        map.once('load', () => {
          if (disposed) return;
          try {
            addSourceAndLayers(map, parsed.collection);
            const settlementSource = map.getSource(SETTLEMENT_SOURCE_ID) as GeoJSONSource;
            const initialSettlements = latestRef.current.settlements;
            settlementIdsRef.current = new Set(initialSettlements.map((entry) => entry.id));
            settlementSource.setData(settlementCollection(initialSettlements));
            syncSettlementVisibility();
            map.on('zoom', syncSettlementVisibility);
            syncFeatureStates();
            fitBounds(provinceBounds, false);
            appliedViewRef.current = 'province';
            map.on('mousemove', SETTLEMENT_DOT_LAYER_ID, handleSettlementMove);
            map.on('mouseleave', SETTLEMENT_DOT_LAYER_ID, clearSettlementHover);
            map.on('click', SETTLEMENT_DOT_LAYER_ID, handleSettlementClick);
            map.on('mousemove', SETTLEMENT_LABEL_LAYER_ID, handleSettlementMove);
            map.on('mouseleave', SETTLEMENT_LABEL_LAYER_ID, clearSettlementHover);
            map.on('click', SETTLEMENT_LABEL_LAYER_ID, handleSettlementClick);
            map.on('mousemove', DISTRICT_FILL_LAYER_ID, handleMapMove);
            map.on('mouseleave', DISTRICT_FILL_LAYER_ID, clearMapHover);
            map.on('click', DISTRICT_FILL_LAYER_ID, handleMapClick);
            canvas.addEventListener('mouseleave', clearHovers);
            setStatus('ready');
          } catch (error) {
            reportMapError({ error });
          }
        });
        map.on('error', reportMapError);
        map.once('remove', () => canvas.removeEventListener('mouseleave', clearHovers));

        let width = container.clientWidth;
        let height = container.clientHeight;
        resizeObserver = new ResizeObserver(() => {
          const nextWidth = container.clientWidth;
          const nextHeight = container.clientHeight;
          if (nextWidth === width && nextHeight === height) return;
          width = nextWidth;
          height = nextHeight;
          map.resize();
          const settlementId = latestRef.current.selectedSettlementId;
          const settlement =
            settlementId === null
              ? null
              : (latestRef.current.settlements.find((entry) => entry.id === settlementId) ?? null);
          if (settlement) {
            map.setCenter([settlement.longitude, settlement.latitude]);
            return;
          }
          const selected = latestRef.current.selectedDistrictId;
          const bounds =
            selected !== null ? districtBoundsRef.current.get(selected) : provinceBoundsRef.current;
          if (bounds) fitBounds(bounds, false, selected !== null ? 9 : undefined);
          // The reveal zoom follows the viewport, so a resize can change it without
          // changing the zoom level itself.
          syncSettlementVisibility();
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
      resizeObserver?.disconnect();
      mapRef.current?.remove();
      mapRef.current = null;
    };
  }, [
    fitBounds,
    focusSettlement,
    guberniaId,
    guberniaName,
    setHover,
    setSettlementHover,
    syncFeatureStates,
    syncSettlementVisibility,
  ]);

  const provinceSelected = selectedDistrictId === null && selectedSettlementId === null;
  const selectedSettlementName =
    selectedSettlementId === null
      ? null
      : (settlements.find((entry) => entry.id === selectedSettlementId)?.name ?? null);
  const selectedDistrictName =
    selectedDistrictId === null
      ? null
      : (districts.find((district) => district.id === selectedDistrictId)?.name ?? null);

  return (
    <section className={styles.shell} aria-label={`Историческая карта: ${guberniaName}`}>
      <div className={styles.viewport}>
        <div ref={containerRef} className={styles.map} />

        <aside className={styles.panel} aria-label="Список уездов">
          <div className={styles.panelHeading}>
            <h2>ДОСТУПНЫЕ УЕЗДЫ</h2>
            <button
              className={styles.provinceButton}
              type="button"
              aria-label="Показать всю губернию"
              title="Показать всю губернию"
              aria-pressed={provinceSelected}
              disabled={status !== 'ready'}
              onClick={() => {
                onDistrictSelect(null);
                if (provinceBoundsRef.current) {
                  appliedViewRef.current = 'province';
                  fitBounds(provinceBoundsRef.current, true);
                }
              }}
            >
              <svg
                aria-hidden="true"
                focusable="false"
                viewBox="0 0 16 16"
                width="14"
                height="14"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M6 1.75H3.25A1.5 1.5 0 0 0 1.75 3.25V6" />
                <path d="M10 1.75h2.75a1.5 1.5 0 0 1 1.5 1.5V6" />
                <path d="M6 14.25H3.25a1.5 1.5 0 0 1-1.5-1.5V10" />
                <path d="M10 14.25h2.75a1.5 1.5 0 0 0 1.5-1.5V10" />
                <circle cx="8" cy="8" r="1.9" fill="currentColor" stroke="none" />
              </svg>
            </button>
          </div>
          <ul className={styles.list} aria-busy={status === 'loading'}>
            {districts.map((district) => {
              const isHovered = hoveredId === district.id;
              const isSelected = selectedDistrictId === district.id;
              return (
                <li key={district.id}>
                  <button
                    className={`${styles.districtButton} ${
                      isHovered ? styles.districtButtonHovered : ''
                    } ${isSelected ? styles.districtButtonSelected : ''}`}
                    type="button"
                    aria-pressed={isSelected}
                    disabled={status !== 'ready'}
                    onMouseEnter={() => setHover(district.id, 'list')}
                    onMouseLeave={(event) => {
                      if (event.currentTarget !== document.activeElement) setHover(null, 'list');
                    }}
                    onFocus={() => setHover(district.id, 'list')}
                    onBlur={() => setHover(null, 'list')}
                    onClick={() => onDistrictSelect(district.id)}
                  >
                    {district.name}
                  </button>
                </li>
              );
            })}
          </ul>
        </aside>

        {status === 'loading' ? (
          <div className={styles.statePanel} role="status">
            <span className={styles.loader} aria-hidden="true" />
            Загружаем границы уездов…
          </div>
        ) : null}
        {status === 'error' ? (
          <div className={styles.errorPanel} role="alert">
            <strong>Карта уездов недоступна</strong>
            <span>{errorMessage}</span>
          </div>
        ) : null}
      </div>

      <span className={styles.visuallyHidden} role="status">
        {status === 'ready'
          ? selectedSettlementName !== null
            ? `Выбран населённый пункт ${selectedSettlementName}.`
            : selectedDistrictName !== null
              ? `Выбран ${selectedDistrictName}.`
              : `Показана вся губерния. Уездов: ${districts.length}, населённых пунктов: ${settlements.length}.`
          : ''}
      </span>
    </section>
  );
}
