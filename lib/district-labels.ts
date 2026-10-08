import polylabel from "polylabel";
import type { Feature, FeatureCollection, MultiPolygon, Point, Polygon, Position } from "geojson";

type NamedDistrict = Feature<Polygon | MultiPolygon, { id: string; name: string }>;

type LabelCollection = FeatureCollection<Point, { id: string; name: string }>;

function ringArea(ring: Position[]): number {
  let twiceArea = 0;
  for (let index = 0, previous = ring.length - 1; index < ring.length; previous = index++) {
    twiceArea += ring[previous][0] * ring[index][1] - ring[index][0] * ring[previous][1];
  }
  return Math.abs(twiceArea) / 2;
}

function polygonArea(polygon: Position[][]): number {
  let area = ringArea(polygon[0]);
  for (let index = 1; index < polygon.length; index++) area -= ringArea(polygon[index]);
  return Math.max(0, area);
}

/** One point inside the largest landmass, even for fragmented districts and holes. */
export function districtLabelCollection(districts: readonly NamedDistrict[]): LabelCollection {
  return {
    type: "FeatureCollection",
    features: districts.map((district) => {
      const geometry = district.geometry;
      let polygon = geometry.type === "Polygon" ? geometry.coordinates : geometry.coordinates[0];
      if (geometry.type === "MultiPolygon") {
        let largestArea = polygonArea(polygon);
        for (let index = 1; index < geometry.coordinates.length; index++) {
          const candidate = geometry.coordinates[index];
          const area = polygonArea(candidate);
          if (area > largestArea) {
            largestArea = area;
            polygon = candidate;
          }
        }
      }
      const [longitude, latitude] = polylabel(polygon as [number, number][][], 0.01);
      return {
        type: "Feature" as const,
        id: district.properties.id,
        properties: { id: district.properties.id, name: district.properties.name },
        geometry: { type: "Point" as const, coordinates: [longitude, latitude] },
      };
    }),
  };
}
