import Link from "next/link";
import { notFound } from "next/navigation";
import SiteHeader from "@/components/SiteHeader";
import { hasAdminSession } from "@/lib/admin-session";
import { getPublicationMetrics } from "@/lib/gubernia-publications";
import styles from "./page.module.css";

export default async function MetricsPage() {
  // Guests must never reach the statistics: reject before reading the state.
  const isAdmin = await hasAdminSession();
  if (!isAdmin) notFound();

  const metrics = await getPublicationMetrics();
  const unpublishedCount = metrics.totalHistoricalGubernias - metrics.publishedCount;

  return (
    <>
      <SiteHeader isAdmin={isAdmin} />
      <main className="content-page content-page--with-header">
        <Link className={`content-page__back ${styles.back}`} href="/">
          ← Вернуться к карте
        </Link>
        <h1 className="content-page__title">Метрики</h1>
        <p className="content-page__text">
          Показатели содержимого портала: опубликованные губернии и созданные в них материалы.
          Статистика посещений здесь не собирается.
        </p>

        <dl className={styles.summary}>
          <div className={styles.summaryItem}>
            <dt className={styles.summaryLabel}>Опубликовано</dt>
            <dd className={styles.summaryValue}>
              {metrics.publishedCount}
              <span className={styles.summaryNote}>
                из {metrics.totalHistoricalGubernias} в коллекции
              </span>
            </dd>
          </div>
          <div className={styles.summaryItem}>
            <dt className={styles.summaryLabel}>Не опубликовано</dt>
            <dd className={styles.summaryValue}>
              {unpublishedCount}
              <span className={styles.summaryNote}>
                из {metrics.totalHistoricalGubernias} в коллекции
              </span>
            </dd>
          </div>
          <div className={styles.summaryItem}>
            <dt className={styles.summaryLabel}>Сообщений</dt>
            <dd className={styles.summaryValue}>
              {metrics.totalPublishedPosts}
              <span className={styles.summaryNote}>в опубликованных губерниях</span>
            </dd>
          </div>
          <div className={styles.summaryItem}>
            <dt className={styles.summaryLabel}>Населённых пунктов</dt>
            <dd className={styles.summaryValue}>
              {metrics.totalPublishedSettlements}
              <span className={styles.summaryNote}>в опубликованных губерниях</span>
            </dd>
          </div>
        </dl>

        <section className={styles.section} aria-labelledby="metrics-provinces">
          <h2 className={styles.sectionTitle} id="metrics-provinces">
            Опубликованные губернии
          </h2>
          {metrics.provinces.length === 0 ? (
            <p className={styles.empty}>
              Пока ни одна губерния не опубликована. Показатели появятся после первой публикации.
            </p>
          ) : (
            <ul className={styles.list}>
              {metrics.provinces.map((province) => (
                <li className={styles.row} key={province.id}>
                  <Link
                    className={styles.province}
                    href={`/guberniya/${encodeURIComponent(province.slug)}`}
                  >
                    {province.name}
                  </Link>
                  <span className={styles.counts}>
                    <span>Сообщений: {province.postsCount}</span>
                    <span>Населённых пунктов: {province.settlementsCount}</span>
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
      </main>
    </>
  );
}
