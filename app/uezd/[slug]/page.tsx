import Link from "next/link";
import { notFound } from "next/navigation";
import SiteHeader from "@/components/SiteHeader";
import { hasAdminSession } from "@/lib/admin-session";
import {
  getPublishedDistrictBySlug,
  getPublishedGeoData,
} from "@/lib/gubernia-publications";
import { normalizePostHighlightQuery } from "@/lib/post-search-highlight";
import { getSiteSettings } from "@/lib/site-settings";
import DistrictAdminControls from "./DistrictAdminControls";
import DistrictExplorer from "./DistrictExplorer";
import styles from "./page.module.css";

// Publication and district addresses live in the backend and must be resolved
// anew for every request, including after an editorial URL change.
export const dynamic = "force-dynamic";

export default async function DistrictPage({
  params,
  searchParams,
}: {
  params: Promise<{ slug: string }>;
  searchParams: Promise<{
    from?: string | string[];
    highlight?: string | string[];
  }>;
}) {
  const { slug } = await params;
  const { from, highlight } = await searchParams;
  const district = await getPublishedDistrictBySlug(slug);
  if (!district) notFound();

  const [geoData, isAdmin, settings] = await Promise.all([
    getPublishedGeoData(),
    hasAdminSession(),
    getSiteSettings(),
  ]);
  const highlightQuery = typeof highlight === "string"
    ? normalizePostHighlightQuery(highlight)
    : "";
  const cameFromSettlements = from === "settlements";
  const provinceHref = `/guberniya/${encodeURIComponent(district.gubernia.slug)}`;
  const backHref = cameFromSettlements
    ? "/?view=settlements"
    : `${provinceHref}#province-map`;
  const provinceBreadcrumbHref = cameFromSettlements
    ? `${provinceHref}?from=settlements`
    : provinceHref;
  const hasPosts = district.posts.length > 0;

  return (
    <>
      <SiteHeader isAdmin={isAdmin} />
      <main className={`${styles.page} ${hasPosts ? styles.withPosts : ""}`}>
        <header className={styles.header} id="district-page-header">
          <Link className={`content-page__back ${styles.back}`} href={backHref}>
            ← Вернуться к карте
          </Link>
          <nav className={styles.breadcrumbs} aria-label="Хлебные крошки">
            <Link href={provinceBreadcrumbHref}>{district.gubernia.name}</Link>
            <span aria-hidden="true">/</span>
            <span aria-current="page">{district.name}</span>
          </nav>
          <p className={`content-page__eyebrow ${styles.eyebrow}`}>
            Историко-генеалогический портал Дмитрия Воробьева
          </p>
          <h1 className={`content-page__title ${styles.title}`}>{district.name}</h1>
        </header>

        {isAdmin ? (
          <div className={styles.adminSettings}>
            <DistrictAdminControls
              guberniaId={district.guberniaId}
              guberniaName={district.gubernia.name}
              districtId={district.id}
              districtName={district.name}
              url={district.url}
              description={district.description}
              images={district.images}
            />
          </div>
        ) : null}

        <DistrictExplorer
          district={district}
          provinces={geoData.provinces}
          editorSettlements={geoData.settlements}
          categories={settings.categories}
          settlementTypes={settings.settlementTypes}
          isAdmin={isAdmin}
          backHref={backHref}
          highlightQuery={highlightQuery}
        />
      </main>
    </>
  );
}
