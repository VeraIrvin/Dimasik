"use client";

import { useEffect, useState } from "react";

export type DistrictOption = {
  id: string;
  name: string;
};

type DistrictState = {
  provinceId: string;
  districts: DistrictOption[];
  loading: boolean;
  error: string;
};

export function useDistrictOptions(guberniaId: string): DistrictState {
  const [state, setState] = useState<DistrictState>({
    provinceId: "",
    districts: [],
    loading: false,
    error: "",
  });

  useEffect(() => {
    if (!guberniaId) {
      setState({ provinceId: "", districts: [], loading: false, error: "" });
      return;
    }

    const controller = new AbortController();
    setState({ provinceId: guberniaId, districts: [], loading: true, error: "" });

    void (async () => {
      try {
        const response = await fetch(
          `/data/uyezds-1897/${encodeURIComponent(guberniaId)}.geojson`,
          { signal: controller.signal },
        );
        if (!response.ok) throw new Error("Не удалось загрузить список уездов.");
        const raw = (await response.json()) as {
          type?: unknown;
          features?: Array<{ properties?: Record<string, unknown> | null }>;
        };
        if (raw.type !== "FeatureCollection" || !Array.isArray(raw.features)) {
          throw new Error("Файл уездов имеет неверный формат.");
        }
        const districts = raw.features.slice(1).map((feature) => {
          const properties = feature?.properties;
          if (
            !properties ||
            properties.kind !== "district" ||
            properties.provinceId !== guberniaId ||
            typeof properties.id !== "string" ||
            !properties.id ||
            typeof properties.name !== "string" ||
            !properties.name.trim()
          ) {
            throw new Error("Файл уездов имеет неверный формат.");
          }
          return { id: properties.id, name: properties.name.trim() };
        });
        districts.sort((left, right) => left.name.localeCompare(right.name, "ru"));
        if (controller.signal.aborted) return;
        setState({ provinceId: guberniaId, districts, loading: false, error: "" });
      } catch (loadError) {
        if (controller.signal.aborted) return;
        setState({
          provinceId: guberniaId,
          districts: [],
          loading: false,
          error: loadError instanceof Error ? loadError.message : "Не удалось загрузить список уездов.",
        });
      }
    })();

    return () => controller.abort();
  }, [guberniaId]);

  return state.provinceId === guberniaId
    ? state
    : { provinceId: guberniaId, districts: [], loading: Boolean(guberniaId), error: "" };
}
