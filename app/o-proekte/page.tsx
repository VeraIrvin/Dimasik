import Image from "next/image";
import Link from "next/link";
import SiteHeader from "@/components/SiteHeader";
import { hasAdminSession } from "@/lib/admin-session";
import { getAboutContent } from "@/lib/about-content";
import { getPublicationMetrics } from "@/lib/gubernia-publications";
import AboutContentEditor from "./AboutContentEditor";
import styles from "./page.module.css";

export default async function AboutPage() {
  const [body, isAdmin, metrics] = await Promise.all([
    getAboutContent(),
    hasAdminSession(),
    getPublicationMetrics(),
  ]);
  const unpublishedCount = metrics.totalHistoricalGubernias - metrics.publishedCount;

  return (
    <>
      <SiteHeader isAdmin={isAdmin} />
      <main className={`content-page content-page--with-header ${styles.page}`}>
        <Link className={`content-page__back ${styles.back}`} href="/">
          ← Вернуться к карте
        </Link>
        <h1 className={`content-page__title ${styles.title}`}>О проекте</h1>

        <div className={styles.introduction}>
          <div className={styles.portraitFrame}>
            <Image
              className={styles.portrait}
              src="/project-portrait.jpg"
              width={981}
              height={1602}
              sizes="(max-width: 800px) min(340px, calc(100vw - 48px)), 320px"
              alt="Портрет на набережной"
            />
          </div>
          <div className={styles.aboutContent}>
            <AboutContentEditor initialBody={body} isAdmin={isAdmin} />
          </div>
        </div>

        <section className={styles.contentBlock} aria-labelledby="portal-content">
          <h2 id="portal-content">Содержимое портала</h2>
          <p className="content-page__text">
            Опубликованные губернии и созданные в них материалы.
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

          <section className={styles.section} aria-labelledby="portal-provinces">
            <h2 className={styles.sectionTitle} id="portal-provinces">
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
        </section>

        <footer className={styles.credits} aria-label="Источники данных и авторство">
          <p className={styles.sourceIntro}>
            Для создания интерактивной карты использованы открытые историко-географические данные
            о границах административно-территориальных единиц Российской империи:
          </p>
          <ol className={styles.sources}>
            <li className={styles.source}>
              <span className={styles.sourceNumber} aria-hidden="true">01</span>
              <div>
                <p className={styles.sourceAuthors}>Sablin, Ivan et al.</p>
                <cite>Transcultural Empire: Geographic Information System of the 1897 and 1926
                  General Censuses in the Russian Empire and Soviet Union</cite>
                <p className={styles.sourceMeta}>
                  heiDATA, Heidelberg University · DOI:{" "}
                  <a href="https://doi.org/10.11588/DATA/10064" target="_blank" rel="noopener noreferrer">
                    10.11588/DATA/10064 ↗
                  </a>
                </p>
              </div>
            </li>
            <li className={styles.source}>
              <span className={styles.sourceNumber} aria-hidden="true">02</span>
              <div>
                <p className={styles.sourceAuthors}>Kessler, Gijs; Markevich, Andrei</p>
                <cite>Russian Empire Historical GIS Maps (1897)</cite>
                <p className={styles.sourceMeta}>
                  Electronic Repository of Russian Historical Statistics (RiStat),
                  International Institute of Social History (IISH).
                  <a
                    className={styles.sourceDataLink}
                    href="https://doi.org/10.34894/NQOASN"
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    Данные RiStat ↗
                  </a>
                </p>
              </div>
            </li>
          </ol>
          <div className={styles.author}>
            <p>
              Концепция, дизайн и разработка сервиса —{" "}
              <a href="https://t.me/verairvin" target="_blank" rel="noopener noreferrer">
                Вера Ирвин ↗
              </a>
            </p>
          </div>
        </footer>
      </main>
    </>
  );
}
