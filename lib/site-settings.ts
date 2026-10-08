import { readRequiredBackendJson } from "@/lib/backend";

export type SiteSettings = {
  categories: string[];
  settlementTypes: string[];
};

export type SiteSettingKind = keyof SiteSettings;

export function getSiteSettings(): Promise<SiteSettings> {
  return readRequiredBackendJson<SiteSettings>("/internal/settings");
}
