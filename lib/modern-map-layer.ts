import type { ExpressionSpecification, FillLayerSpecification, Map as MapLibreMap } from 'maplibre-gl';

const SOURCE_ID = 'modern-openstreetmap';
const LAYER_ID = 'modern-openstreetmap-raster';
const TILE_URL = 'https://tile.openstreetmap.org/{z}/{x}/{y}.png';
const ATTRIBUTION = '© <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noopener noreferrer">OpenStreetMap</a> contributors';

type FillOpacity = NonNullable<FillLayerSpecification['paint']>['fill-opacity'];
type FillSnapshot = { id: string; original: FillOpacity; modern: FillOpacity };

export function isModernMapError(event: unknown): boolean {
  if (typeof event === 'string') return event.includes('https://tile.openstreetmap.org/');
  if (typeof event !== 'object' || event === null) return false;
  if ('sourceId' in event && event.sourceId === SOURCE_ID) return true;
  if ('message' in event && typeof event.message === 'string' && event.message.includes('https://tile.openstreetmap.org/')) {
    return true;
  }
  if (!('error' in event) || typeof event.error !== 'object' || event.error === null) return false;
  const error = event.error;
  return (
    ('sourceId' in error && error.sourceId === SOURCE_ID) ||
    ('message' in error && typeof error.message === 'string' && error.message.includes('https://tile.openstreetmap.org/'))
  );
}

// Keep zoom curves at the top level: wrapping an entire zoom-dependent expression
// in multiplication would violate the MapLibre expression specification.
function scaleExpression(value: unknown, factor: number): unknown {
  if (typeof value === 'number') return value * factor;
  if (value === null) return null;
  if (Array.isArray(value)) {
    const expression = [...value];
    if (value[0] === 'interpolate') {
      for (let index = 4; index < expression.length; index += 2) {
        expression[index] = scaleExpression(expression[index], factor);
      }
      return expression;
    }
    if (value[0] === 'step') {
      for (let index = 2; index < expression.length; index += 2) {
        expression[index] = scaleExpression(expression[index], factor);
      }
      return expression;
    }
    if (value[0] === 'let') {
      expression[expression.length - 1] = scaleExpression(expression[expression.length - 1], factor);
      return expression;
    }
    if (value[0] === 'coalesce') {
      for (let index = 1; index < expression.length; index += 1) {
        expression[index] = scaleExpression(expression[index], factor);
      }
      return expression;
    }
  }
  return ['*', value, factor];
}

function modernOpacity(original: FillOpacity): FillOpacity {
  const factor = original === undefined ? 0.24 : 0.28;
  if (original === undefined) return factor;
  if (typeof original === 'number') return original * factor;
  if (Array.isArray(original)) return scaleExpression(original, factor) as ExpressionSpecification;
  // Preserve legacy function stops too, without mutating the saved function.
  if ('stops' in original) {
    return {
      ...original,
      stops: original.stops.map(([input, output]) => [input, output * factor]),
      default: ('default' in original && typeof original.default === 'number' ? original.default : 1) * factor,
    } as FillOpacity;
  }
  return ['*', ['number', ['get', original.property], original.default ?? 1], factor];
}

export class ModernMapLayerController {
  private readonly map: MapLibreMap;
  private fills: FillSnapshot[] | null = null;
  private initialized = false;
  enabled = false;

  constructor(map: MapLibreMap) {
    this.map = map;
  }

  private initialize() {
    if (this.initialized) return;
    const layers = this.map.getStyle().layers;
    if (this.fills === null) {
      this.fills = layers.filter((layer) => layer.type === 'fill').map((layer) => {
        const original = this.map.getPaintProperty(layer.id, 'fill-opacity');
        return { id: layer.id, original, modern: modernOpacity(original) };
      });
    }
    if (!this.map.getSource(SOURCE_ID)) {
      this.map.addSource(SOURCE_ID, {
        type: 'raster',
        tiles: [TILE_URL],
        minzoom: 0,
        maxzoom: 19,
        tileSize: 256,
        attribution: ATTRIBUTION,
      });
    }
    if (!this.map.getLayer(LAYER_ID)) {
      const firstHistoricalLayer = layers.find((layer) => layer.type !== 'background');
      this.map.addLayer({
        id: LAYER_ID,
        type: 'raster',
        source: SOURCE_ID,
        layout: { visibility: 'none' },
      }, firstHistoricalLayer?.id);
    }
    this.initialized = true;
  }

  setEnabled(enabled: boolean) {
    if (enabled === this.enabled) return;
    if (enabled) this.initialize();
    this.enabled = enabled;
    if (!enabled && this.map.getLayer(LAYER_ID)) {
      this.map.setLayoutProperty(LAYER_ID, 'visibility', 'none');
    }
    for (const fill of this.fills ?? []) {
      if (this.map.getLayer(fill.id)) {
        // undefined unsets an originally absent property, restoring its default.
        this.map.setPaintProperty(fill.id, 'fill-opacity', enabled ? fill.modern : fill.original);
      }
    }
    if (enabled) this.map.setLayoutProperty(LAYER_ID, 'visibility', 'visible');
  }
}

// Retain the original snapshots and once-added source/layer if React reattaches
// the hook to the same map (including development Strict Mode effect replay).
const controllers = new WeakMap<MapLibreMap, ModernMapLayerController>();

export function getModernMapLayerController(map: MapLibreMap): ModernMapLayerController {
  let controller = controllers.get(map);
  if (!controller) {
    controller = new ModernMapLayerController(map);
    controllers.set(map, controller);
  }
  return controller;
}
