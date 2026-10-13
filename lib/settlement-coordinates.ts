import type { Settlement } from "@/lib/gubernia-publications";

type SettlementCoordinates = Pick<Settlement, "latitude" | "longitude">;

const COORDINATES_ERROR =
  "Укажите координаты в формате «широта, долгота», например 56.1234, 40.5678.";

/** Only complete, finite coordinate pairs can become map points. */
export function hasSettlementCoordinates(
  settlement: SettlementCoordinates,
): settlement is { latitude: number; longitude: number } {
  return (
    typeof settlement.latitude === "number" &&
    Number.isFinite(settlement.latitude) &&
    settlement.latitude >= -90 &&
    settlement.latitude <= 90 &&
    typeof settlement.longitude === "number" &&
    Number.isFinite(settlement.longitude) &&
    settlement.longitude >= -180 &&
    settlement.longitude <= 180
  );
}

/** Empty text keeps coordinate editing enabled for settlements without a point. */
export function formatSettlementCoordinates(settlement: SettlementCoordinates): string {
  return hasSettlementCoordinates(settlement)
    ? `${settlement.latitude}, ${settlement.longitude}`
    : "";
}

/** Optional input; nonempty values must contain a valid "latitude, longitude" pair. */
export function parseSettlementCoordinates(value: string): [number, number] | null {
  if (!value.trim()) return null;
  const parts = value.split(",");
  if (parts.length !== 2 || !parts[0].trim() || !parts[1].trim()) {
    throw new Error(COORDINATES_ERROR);
  }
  const latitude = Number(parts[0].trim());
  const longitude = Number(parts[1].trim());
  if (!hasSettlementCoordinates({ latitude, longitude })) {
    throw new Error(COORDINATES_ERROR);
  }
  return [latitude, longitude];
}
