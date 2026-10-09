import Link from "next/link";
import { notFound } from "next/navigation";
import SiteHeader from "@/components/SiteHeader";
import { hasAdminSession } from "@/lib/admin-session";
import { getPublishedGeoData, getPublishedGuberniaBySlug } from "@/lib/gubernia-publications";
import { getSiteSettings } from "@/lib/site-settings";
import GuberniaAdminControls from "./GuberniaAdminControls";
import ProvinceExplorer from "./ProvinceExplorer";
import styles from "./page.module.css";

export default async function GuberniaPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const [gubernia, geoData, isAdmin, settings] = await Promise.all([
    getPublishedGuberniaBySlug(slug),
    getPublishedGeoData(),
    hasAdminSession(),
    getSiteSettings(),
  ]);

  if (!gubernia) {
    notFound();
  }

  const hasPosts = gubernia.posts.length > 0;

  return (
    <>
      <SiteHeader isAdmin={isAdmin} />
      <main className={`${styles.page} ${hasPosts ? styles.withPosts : ""}`}>
        <header className={styles.header} id="province-page-header">
          <Link className={`content-page__back ${styles.back}`} href="/">
            ← Вернуться к карте
          </Link>
          <p className={`content-page__eyebrow ${styles.eyebrow}`}>
            Историко-генеалогический портал Дмитрия Воробьева
          </p>
          <h1 className={`content-page__title ${styles.title}`}>{gubernia.name}</h1>
        </header>

        {isAdmin ? (
          <div className={styles.adminSettings}>
            <GuberniaAdminControls
              id={gubernia.id}
              slug={gubernia.slug}
              description={gubernia.description}
              images={gubernia.images}
            />
          </div>
        ) : null}

        <ProvinceExplorer
          gubernia={gubernia}
          provinces={geoData.provinces}
          settlements={geoData.settlements}
          categories={settings.categories}
          settlementTypes={settings.settlementTypes}
          isAdmin={isAdmin}
        />
      </main>
    </>
  );
}
