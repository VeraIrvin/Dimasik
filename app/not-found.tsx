import type { Metadata } from "next";
import Image from "next/image";
import Link from "next/link";
import { connection } from "next/server";
import greenBottomLeft from "../public/404/green-bottom-left.webp";
import greenTitle from "../public/404/green-title.webp";
import greenTopRight from "../public/404/green-top-right.webp";
import redBottomLeft from "../public/404/red-bottom-left.webp";
import redBottomRight from "../public/404/red-bottom-right.webp";
import redTitle from "../public/404/red-title.webp";
import redTopRight from "../public/404/red-top-right.webp";
import sceneRed from "../public/404/scene-red.webp";
import sceneGreen from "../public/404/scene-green.webp";
import styles from "./not-found.module.css";

export const metadata: Metadata = {
  title: "Страница не найдена — Дмитрий Воробьев",
  description:
    "Такой страницы нет. Вернитесь на главную к исторической карте 76 губерний.",
};

const RED_SCENE_ALT =
  "Рукописная иллюстрация: фигура в красном, два демона и цветочный орнамент";

const GREEN_SCENE_ALT =
  "Рукописная иллюстрация: фигура с мечом и короной среди листвы";

export default async function NotFound() {
  // Randomness must happen per request, not during prerendering.
  await connection();
  const isRed = Math.random() < 0.5;

  return (
    <div
      className={`${styles.page} ${
        isRed ? styles.variantRed : styles.variantGreen
      }`}
    >
      {isRed ? (
        <>
          <main className={styles.redMain}>
            <div className={styles.redCopy}>
              <h1 className={styles.visuallyHidden}>
                404. Страница не найдена
              </h1>
              <p className={styles.visuallyHidden}>
                Похоже, вы свернули не туда.
              </p>
              <Image
                className={styles.redTitle}
                src={redTitle}
                alt=""
                sizes="(max-width: 640px) calc(100vw - 40px), (max-width: 860px) 480px, 36vw"
              />
              <p className={styles.redActions}>
                <Link className={styles.redButton} href="/">
                  Вернуться на главную
                  <span aria-hidden="true" className={styles.redButtonArrow}>
                    →
                  </span>
                </Link>
              </p>
            </div>
            <figure className={styles.redScene}>
              <Image
                className={styles.redSceneImage}
                src={sceneRed}
                alt={RED_SCENE_ALT}
                sizes="(max-width: 640px) calc(100vw - 48px), (max-width: 860px) 460px, 60vw"
              />
            </figure>
          </main>
          <Image
            className={`${styles.redCorner} ${styles.redBottomLeft}`}
            src={redBottomLeft}
            alt=""
            sizes="(max-width: 640px) 68vw, 53vw"
          />
          <Image
            className={`${styles.redCorner} ${styles.redBottomRight}`}
            src={redBottomRight}
            alt=""
            sizes="(max-width: 640px) 25vw, 22vw"
          />
          <Image
            className={`${styles.redCorner} ${styles.redTopRight}`}
            src={redTopRight}
            alt=""
            sizes="(max-width: 640px) 22vw, 20vw"
          />
        </>
      ) : (
        <>
          <main className={styles.greenMain}>
            <div className={styles.greenCopy}>
              <h1 className={styles.visuallyHidden}>
                404. Страница не найдена
              </h1>
              <p className={styles.visuallyHidden}>
                Похоже, вы свернули не туда.
              </p>
              <Image
                className={styles.greenTitle}
                src={greenTitle}
                alt=""
                sizes="(max-width: 640px) calc(100vw - 32px), (max-width: 860px) 656px, 46vw"
              />
              <p className={styles.greenActions}>
                <Link className={styles.greenButton} href="/">
                  Вернуться на главную
                </Link>
              </p>
            </div>
            <figure className={styles.greenScene}>
              <Image
                className={styles.greenSceneImage}
                src={sceneGreen}
                alt={GREEN_SCENE_ALT}
                sizes="(max-width: 640px) calc(100vw - 32px), (max-width: 860px) 680px, 61vw"
              />
            </figure>
          </main>
          <Image
            className={`${styles.greenCorner} ${styles.greenBottomLeft}`}
            src={greenBottomLeft}
            alt=""
            sizes="(max-width: 860px) min(46vw, 260px), 34vw"
          />
          <Image
            className={`${styles.greenCorner} ${styles.greenTopRight}`}
            src={greenTopRight}
            alt=""
            sizes="(max-width: 860px) min(26vw, 130px), 20vw"
          />
        </>
      )}
    </div>
  );
}
