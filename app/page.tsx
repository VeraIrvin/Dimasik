import GuberniasMap from "@/components/GuberniasMap";
import SiteHeader from "@/components/SiteHeader";
import { hasAdminSession } from "@/lib/admin-session";
import { getPublishedGeoData } from "@/lib/gubernia-publications";
import { getSiteSettings } from "@/lib/site-settings";

export default async function HomePage({
  searchParams,
}: {
  searchParams: Promise<{ view?: string | string[] }>;
}) {
  const [isAdmin, geoData, settings, { view }] = await Promise.all([
    hasAdminSession(),
    getPublishedGeoData(),
    getSiteSettings(),
    searchParams,
  ]);
  // Only the settlement deep link is meaningful; a missing or unknown value
  // keeps the governorate overview.
  const initialView = view === "settlements" ? "settlements" : "gubernias";
  return (
    <main className="site-shell">
      <SiteHeader isAdmin={isAdmin} />
      <GuberniasMap
        isAdmin={isAdmin}
        settings={settings}
        settlements={geoData.settlements}
        provinces={geoData.provinces}
        initialView={initialView}
      />
    </main>
  );
}
