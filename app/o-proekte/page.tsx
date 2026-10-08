import Link from "next/link";
import { hasAdminSession } from "@/lib/admin-session";
import { getAboutContent } from "@/lib/about-content";
import AboutContentEditor from "./AboutContentEditor";
import styles from "./page.module.css";

export default async function AboutPage() {
  const [body, isAdmin] = await Promise.all([getAboutContent(), hasAdminSession()]);

  return (
    <main className="content-page">
      <Link className={`content-page__back ${styles.back}`} href="/">
        ← Вернуться к карте
      </Link>
      <p className={`content-page__eyebrow ${styles.eyebrow}`}>
        Историко-генеалогический портал Дмитрия Воробьева
      </p>
      <h1 className="content-page__title">О проекте</h1>

      <AboutContentEditor initialBody={body} isAdmin={isAdmin} />

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
  );
}
