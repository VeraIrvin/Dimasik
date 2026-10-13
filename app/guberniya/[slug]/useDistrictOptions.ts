"use client";

import { useEffect, useState } from "react";
import type { DistrictOption } from "@/lib/gubernia-publications";

type DistrictState = {
  provinceId: string;
  districts: DistrictOption[];
  loading: boolean;
  error: string;
};

export function useDistrictOptions(
  guberniaId: string,
  contextGuberniaId?: string,
  currentDistricts?: DistrictOption[],
): DistrictState {
  const hasProvidedDistricts =
    Boolean(guberniaId) && guberniaId === contextGuberniaId && currentDistricts !== undefined;
  const [state, setState] = useState<DistrictState>({
    provinceId: "",
    districts: [],
    loading: false,
    error: "",
  });

  useEffect(() => {
    if (!guberniaId || hasProvidedDistricts) {
      setState({ provinceId: "", districts: [], loading: false, error: "" });
      return;
    }

    const controller = new AbortController();
    setState({ provinceId: guberniaId, districts: [], loading: true, error: "" });

    void (async () => {
      try {
        const response = await fetch(
          `/api/gubernias/${encodeURIComponent(guberniaId)}/districts`,
          { signal: controller.signal, cache: "no-store" },
        );
        if (!response.ok) throw new Error("Не удалось загрузить список уездов.");
        const raw = (await response.json()) as unknown;
        if (
          typeof raw !== "object" ||
          raw === null ||
          !("districts" in raw) ||
          !Array.isArray(raw.districts)
        ) {
          throw new Error("Список уездов имеет неверный формат.");
        }
        const districts = raw.districts.map((district): DistrictOption => {
          if (
            typeof district !== "object" ||
            district === null ||
            !("id" in district) ||
            typeof district.id !== "string" ||
            !district.id ||
            !("name" in district) ||
            typeof district.name !== "string" ||
            !district.name.trim() ||
            !("guberniaId" in district) ||
            district.guberniaId !== guberniaId ||
            !("url" in district) ||
            typeof district.url !== "string" ||
            !district.url
          ) {
            throw new Error("Список уездов имеет неверный формат.");
          }
          return {
            id: district.id,
            name: district.name.trim(),
            guberniaId: district.guberniaId,
            url: district.url,
          };
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
  }, [guberniaId, hasProvidedDistricts]);

  // Read current server props directly so a refresh changes labels without
  // resetting the editor's selected IDs or any other draft fields.
  if (hasProvidedDistricts && currentDistricts) {
    return { provinceId: guberniaId, districts: currentDistricts, loading: false, error: "" };
  }

  return state.provinceId === guberniaId
    ? state
    : { provinceId: guberniaId, districts: [], loading: Boolean(guberniaId), error: "" };
}
