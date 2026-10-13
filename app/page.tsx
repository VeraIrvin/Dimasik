import HomeMap from "@/components/HomeMap";
import { hasAdminSession } from "@/lib/admin-session";
import { getPublishedGeoData } from "@/lib/gubernia-publications";
import { getSiteSettings } from "@/lib/site-settings";

export default async function HomePage({
  searchParams,
}: {
  searchParams: Promise<{
    view?: string | string[];
    settlement?: string | string[];
  }>;
}) {
  const [isAdmin, geoData, settings, { view, settlement }] = await Promise.all([
    hasAdminSession(),
    getPublishedGeoData(),
    getSiteSettings(),
    searchParams,
  ]);
  // Only the settlement deep link is meaningful; a missing or unknown value
  // keeps the governorate overview.
  const initialView = view === "settlements" ? "settlements" : "gubernias";
  const initialSettlementId =
    initialView === "settlements" && typeof settlement === "string" ? settlement : null;
  return (
    <HomeMap
      isAdmin={isAdmin}
      settings={settings}
      settlements={geoData.settlements}
      provinces={geoData.provinces}
      initialView={initialView}
      initialSettlementId={initialSettlementId}
    />
  );
}
