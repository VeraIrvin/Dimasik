import Link from "next/link";
import Image from "next/image";
import GuberniasMap from "@/components/GuberniasMap";
import AdminAccess from "@/components/AdminAccess";
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
      <header className="site-header">
        <Link className="site-brand" href="/" aria-label="Историко-генеалогический портал Дмитрия Воробьева — на главную">
          <Image className="site-brand__mark" src="/logo-gold.png" alt="" width={52} height={52} priority />
          <span className="site-brand__name">Историко-генеалогический портал Дмитрия Воробьева</span>
        </Link>
        <nav className="site-nav" aria-label="Основная навигация">
          <Link href="/o-proekte">О проекте</Link>
          {isAdmin ? <Link href="/nastroyki">Настройки</Link> : null}
          {isAdmin ? <Link href="/metriki">Метрики</Link> : null}
          <AdminAccess initialIsAdmin={isAdmin} />
        </nav>
      </header>
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
