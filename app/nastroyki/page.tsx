import Link from "next/link";
import { notFound } from "next/navigation";
import SiteHeader from "@/components/SiteHeader";
import { hasAdminSession } from "@/lib/admin-session";
import { getSiteSettings } from "@/lib/site-settings";
import SettingsEditor from "./SettingsEditor";
import styles from "./page.module.css";

export default async function SettingsPage() {
  // Guests must never reach the settings: reject before reading the state.
  const isAdmin = await hasAdminSession();
  if (!isAdmin) notFound();

  const settings = await getSiteSettings();

  return (
    <>
      <SiteHeader isAdmin={isAdmin} />
      <main className="content-page content-page--with-header">
        <Link className={`content-page__back ${styles.back}`} href="/">
          ← Вернуться к карте
        </Link>
        <h1 className="content-page__title">Настройки</h1>
        <p className="content-page__text">
          Списки значений для форм портала. Названия можно добавлять, переименовывать и удалять.
          Изменения влияют на выбор в новых материалах; опубликованные записи и населённые пункты
          сохраняют прежние значения.
        </p>

        <SettingsEditor initialSettings={settings} />
      </main>
    </>
  );
}
