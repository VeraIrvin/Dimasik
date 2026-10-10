import Link from "next/link";
import { notFound } from "next/navigation";
import SiteHeader from "@/components/SiteHeader";
import { hasAdminSession } from "@/lib/admin-session";
import TrafficPanel from "./TrafficPanel";
import styles from "./page.module.css";

export default async function MetricsPage() {
  // Guests must never reach traffic statistics.
  const isAdmin = await hasAdminSession();
  if (!isAdmin) notFound();

  return (
    <>
      <SiteHeader isAdmin={isAdmin} />
      <main className="content-page content-page--with-header">
        <Link className={`content-page__back ${styles.back}`} href="/">
          ← Вернуться к карте
        </Link>
        <h1 className="content-page__title">Метрики</h1>
        <TrafficPanel />
      </main>
    </>
  );
}
