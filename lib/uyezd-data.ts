import { readFile } from "node:fs/promises";
import path from "node:path";

export type Uyezd = {
  id: string;
  name: string;
  provinceId: string;
};

type Position = [number, number];
type PolygonCoordinates = Position[][];
type DistrictGeometry =
  | { type: "Polygon"; coordinates: PolygonCoordinates }
  | { type: "MultiPolygon"; coordinates: PolygonCoordinates[] };

type DistrictFeature = {
  type: "Feature";
  properties: Uyezd & { kind: "district" };
  geometry: DistrictGeometry;
};

type UyezdCollection = {
  districts: DistrictFeature[];
  byId: Map<string, DistrictFeature>;
};

type UyezdRuntime = {
  collections: Map<string, Promise<UyezdCollection>>;
};

const UYEZDS_DIRECTORY = path.join(process.cwd(), "public", "data", "uyezds-1897");
const PROVINCE_ID_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const EMPTY_COLLECTION: UyezdCollection = { districts: [], byId: new Map() };
const runtimeKey = Symbol.for("dimasik.uyezd-data.runtime");
const uyezdGlobal = globalThis as typeof globalThis & {
  [key: symbol]: UyezdRuntime | undefined;
};
const runtime = uyezdGlobal[runtimeKey] ?? { collections: new Map() };
uyezdGlobal[runtimeKey] = runtime;

function isPosition(value: unknown): value is Position {
  return (
    Array.isArray(value) &&
    value.length >= 2 &&
    typeof value[0] === "number" &&
    Number.isFinite(value[0]) &&
    typeof value[1] === "number" &&
    Number.isFinite(value[1])
  );
}

function isRing(value: unknown): value is Position[] {
  return Array.isArray(value) && value.length >= 4 && value.every(isPosition);
}

function isPolygonCoordinates(value: unknown): value is PolygonCoordinates {
  return Array.isArray(value) && value.length > 0 && value.every(isRing);
}

function parseGeometry(value: unknown, districtId: string): DistrictGeometry {
  if (typeof value !== "object" || value === null) {
    throw new Error(`Historical district ${districtId} has invalid geometry.`);
  }
  const geometry = value as { type?: unknown; coordinates?: unknown };
  if (geometry.type === "Polygon" && isPolygonCoordinates(geometry.coordinates)) {
    return { type: "Polygon", coordinates: geometry.coordinates };
  }
  if (
    geometry.type === "MultiPolygon" &&
    Array.isArray(geometry.coordinates) &&
    geometry.coordinates.length > 0 &&
    geometry.coordinates.every(isPolygonCoordinates)
  ) {
    return { type: "MultiPolygon", coordinates: geometry.coordinates };
  }
  throw new Error(`Historical district ${districtId} has unsupported geometry.`);
}

function parseCollection(value: unknown, provinceId: string): UyezdCollection {
  if (typeof value !== "object" || value === null) {
    throw new Error(`Historical district GeoJSON for ${provinceId} is not an object.`);
  }
  const collection = value as { type?: unknown; features?: unknown };
  if (
    collection.type !== "FeatureCollection" ||
    !Array.isArray(collection.features) ||
    collection.features.length === 0
  ) {
    throw new Error(`Historical district GeoJSON for ${provinceId} is invalid.`);
  }

  const [province, ...rawDistricts] = collection.features as unknown[];
  if (typeof province !== "object" || province === null) {
    throw new Error(`Historical district GeoJSON does not match province ${provinceId}.`);
  }
  const provinceFeature = province as { type?: unknown; properties?: unknown };
  if (typeof provinceFeature.properties !== "object" || provinceFeature.properties === null) {
    throw new Error(`Historical district GeoJSON does not match province ${provinceId}.`);
  }
  const provinceProperties = provinceFeature.properties as { kind?: unknown; id?: unknown };
  if (
    provinceFeature.type !== "Feature" ||
    provinceProperties.kind !== "province" ||
    provinceProperties.id !== provinceId
  ) {
    throw new Error(`Historical district GeoJSON does not match province ${provinceId}.`);
  }

  const districts: DistrictFeature[] = [];
  const byId = new Map<string, DistrictFeature>();
  for (const rawFeature of rawDistricts) {
    if (typeof rawFeature !== "object" || rawFeature === null) {
      throw new Error(`Historical district GeoJSON for ${provinceId} contains an invalid feature.`);
    }
    const feature = rawFeature as { type?: unknown; properties?: unknown; geometry?: unknown };
    if (typeof feature.properties !== "object" || feature.properties === null) {
      throw new Error(`Historical district GeoJSON for ${provinceId} contains an invalid feature.`);
    }
    const properties = feature.properties as {
      kind?: unknown;
      id?: unknown;
      name?: unknown;
      provinceId?: unknown;
    };
    if (
      feature.type !== "Feature" ||
      properties.kind !== "district" ||
      typeof properties.id !== "string" ||
      !/^uyezd-1897-\d+$/.test(properties.id) ||
      typeof properties.name !== "string" ||
      properties.name.length === 0 ||
      properties.provinceId !== provinceId
    ) {
      throw new Error(`Historical district GeoJSON for ${provinceId} has invalid metadata.`);
    }
    if (byId.has(properties.id)) {
      throw new Error(`Historical district GeoJSON contains duplicate id ${properties.id}.`);
    }
    const district: DistrictFeature = {
      type: "Feature",
      properties: {
        kind: "district",
        id: properties.id,
        name: properties.name,
        provinceId,
      },
      geometry: parseGeometry(feature.geometry, properties.id),
    };
    districts.push(district);
    byId.set(district.properties.id, district);
  }
  return { districts, byId };
}

async function loadCollection(provinceId: string): Promise<UyezdCollection> {
  if (!PROVINCE_ID_PATTERN.test(provinceId)) {
    throw new Error(`Invalid province id ${provinceId}.`);
  }
  const cached = runtime.collections.get(provinceId);
  if (cached) return cached;

  const loaded = readFile(path.join(UYEZDS_DIRECTORY, `${provinceId}.geojson`), "utf8")
    .then((source) => parseCollection(JSON.parse(source) as unknown, provinceId))
    .catch((error: unknown) => {
      // A province without a generated district file has no valid districts;
      // the failed promise is never cached so a later file appears on retry.
      runtime.collections.delete(provinceId);
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return EMPTY_COLLECTION;
      throw error;
    });
  runtime.collections.set(provinceId, loaded);
  return loaded;
}

export async function getUyezd(provinceId: string, uyezdId: string): Promise<Uyezd | null> {
  const district = (await loadCollection(provinceId)).byId.get(uyezdId);
  if (!district) return null;
  return {
    id: district.properties.id,
    name: district.properties.name,
    provinceId: district.properties.provinceId,
  };
}

function pointOnSegment(point: Position, start: Position, end: Position): boolean {
  const [x, y] = point;
  const [x1, y1] = start;
  const [x2, y2] = end;
  const cross = (x - x1) * (y2 - y1) - (y - y1) * (x2 - x1);
  const tolerance = Number.EPSILON * 32 * Math.max(1, Math.abs(x), Math.abs(y), Math.abs(x1), Math.abs(y1), Math.abs(x2), Math.abs(y2));
  if (Math.abs(cross) > tolerance) return false;
  return (
    x >= Math.min(x1, x2) - tolerance &&
    x <= Math.max(x1, x2) + tolerance &&
    y >= Math.min(y1, y2) - tolerance &&
    y <= Math.max(y1, y2) + tolerance
  );
}

type RingLocation = "outside" | "inside" | "boundary";

function locateInRing(point: Position, ring: Position[]): RingLocation {
  let inside = false;
  for (let index = 0, previous = ring.length - 1; index < ring.length; previous = index++) {
    const start = ring[previous];
    const end = ring[index];
    if (pointOnSegment(point, start, end)) return "boundary";
    const crosses =
      (start[1] > point[1]) !== (end[1] > point[1]) &&
      point[0] < ((end[0] - start[0]) * (point[1] - start[1])) / (end[1] - start[1]) + start[0];
    if (crosses) inside = !inside;
  }
  return inside ? "inside" : "outside";
}

function pointInPolygon(point: Position, polygon: PolygonCoordinates): boolean {
  const outerLocation = locateInRing(point, polygon[0]);
  if (outerLocation === "outside") return false;
  if (outerLocation === "boundary") return true;
  for (const hole of polygon.slice(1)) {
    const holeLocation = locateInRing(point, hole);
    if (holeLocation === "inside") return false;
    if (holeLocation === "boundary") return true;
  }
  return true;
}

export async function isPointInUyezd(
  provinceId: string,
  uyezdId: string,
  longitude: number,
  latitude: number,
): Promise<boolean> {
  const district = (await loadCollection(provinceId)).byId.get(uyezdId);
  if (!district) return false;
  const point: Position = [longitude, latitude];
  return district.geometry.type === "Polygon"
    ? pointInPolygon(point, district.geometry.coordinates)
    : district.geometry.coordinates.some((polygon) => pointInPolygon(point, polygon));
}
