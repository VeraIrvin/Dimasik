import Link from "next/link";
import { notFound } from "next/navigation";
import AdminCreationPanel from "@/app/guberniya/[slug]/AdminCreationPanel";
import SiteHeader from "@/components/SiteHeader";
import { hasAdminSession } from "@/lib/admin-session";
import { getPublishedGeoData, getPublishedSettlementBySlug } from "@/lib/gubernia-publications";
import { getSettlementReference } from "@/lib/settlement-reference";
import { getSiteSettings } from "@/lib/site-settings";
import { getUyezd } from "@/lib/uyezd-data";
import SettlementReferenceEditor from "./SettlementReferenceEditor";
import SettlementPostExplorer from "./SettlementPostExplorer";
import styles from "./page.module.css";

/** Element id of the slot hosting the record's admin row, above the details. */
const SETTLEMENT_ADMIN_SLOT_ID = "settlement-record-admin";

// Publication state lives in the Rust backend, so every request must resolve
// the settlement anew.
export const dynamic = "force-dynamic";

export default async function SettlementPage({
  params,
  searchParams,
}: {
  params: Promise<{ slug: string }>;
  searchParams: Promise<{ from?: string | string[] }>;
}) {
  const { slug } = await params;
  const { from } = await searchParams;
  const published = await getPublishedSettlementBySlug(slug);
  if (!published) {
    notFound();
  }

  const { settlement, gubernia } = published;
  // Districts may not be generated for every province; the row is simply omitted then.
  const [uyeza, reference, geoData, isAdmin, settings] = await Promise.all([
    getUyezd(gubernia.id, settlement.uyezdId),
    getSettlementReference(settlement.id),
    getPublishedGeoData(),
    hasAdminSession(),
    getSiteSettings(),
  ]);

  // Placement validation forces a post's uyezd to match its settlement, so the
  // settlement district is the only name the feed can need.
  const districtNames: Record<string, string> = uyeza ? { [uyeza.id]: uyeza.name } : {};
  const posts = gubernia.posts.filter((post) => post.settlementId === settlement.id);
  const hasPosts = posts.length > 0;
  const backHref = from === "settlements"
    ? "/?view=settlements"
    : from === "province"
      ? `/guberniya/${encodeURIComponent(gubernia.slug)}#province-map`
      : "/";

  return (
    <>
      <SiteHeader isAdmin={isAdmin} />
      <main className={`content-page ${styles.page} ${hasPosts ? styles.withPosts : ""}`}>
        <header className={styles.header} id="settlement-page-header">
          <Link className={`content-page__back ${styles.back}`} href={backHref}>
            ← Вернуться к карте
          </Link>
          <p className={`content-page__eyebrow ${styles.eyebrow}`}>
            Историко-генеалогический портал Дмитрия Воробьева
          </p>
          <h1 className={`content-page__title ${styles.title}`}>{settlement.name}</h1>
        </header>

        {/* Admin row slot: «Редактировать/Удалить населённый пункт» above the details. */}
        <div id={SETTLEMENT_ADMIN_SLOT_ID} className={styles.adminSlot} />

        <dl className={styles.details}>
          {settlement.type ? (
            <div className={styles.detail}>
              <dt className={styles.detailLabel}>Тип населённого пункта</dt>
              <dd className={styles.detailValue}>{settlement.type}</dd>
            </div>
          ) : null}
          {uyeza ? (
            <div className={styles.detail}>
              <dt className={styles.detailLabel}>Уезд</dt>
              <dd className={styles.detailValue}>{uyeza.name}</dd>
            </div>
          ) : null}
          <div className={styles.detail}>
            <dt className={styles.detailLabel}>Губерния</dt>
            <dd className={styles.detailValue}>
              <Link
                className={styles.provinceLink}
                href={`/guberniya/${encodeURIComponent(gubernia.slug)}`}
              >
                {gubernia.name}
              </Link>
            </dd>
          </div>
          <div className={styles.detail}>
            <dt className={styles.detailLabel}>Координаты</dt>
            <dd className={styles.detailValue}>
              {settlement.latitude}, {settlement.longitude}
            </dd>
          </div>
        </dl>

        <section
          id="settlement-reference"
          className={styles.section}
          aria-labelledby="settlement-reference-heading"
        >
          <h2 id="settlement-reference-heading" className={styles.sectionHeading}>
            Справочные сведения
          </h2>
          <SettlementReferenceEditor
            slug={slug}
            initialBody={reference}
            isAdmin={isAdmin}
            settlementId={settlement.id}
            guberniaId={gubernia.id}
            settlementName={settlement.name}
            backHref={backHref}
            adminSlotId={SETTLEMENT_ADMIN_SLOT_ID}
          />
        </section>

        {isAdmin ? (
          <div className={styles.composer}>
            <AdminCreationPanel
              guberniaId={gubernia.id}
              provinces={geoData.provinces}
              settlements={geoData.settlements}
              categories={settings.categories}
              settlementTypes={settings.settlementTypes}
              defaultSettlementId={settlement.id}
            />
          </div>
        ) : null}

        <section className={styles.feedSection} aria-label="Сообщения населённого пункта">
          <SettlementPostExplorer
            posts={posts}
            guberniaId={gubernia.id}
            provinces={geoData.provinces}
            settlements={geoData.settlements}
            categories={settings.categories}
            districtNames={districtNames}
            isAdmin={isAdmin}
            backHref={backHref}
          />
        </section>
      </main>
    </>
  );
}
