import Image from "next/image";
import Link from "next/link";
import AdminAccess from "@/components/AdminAccess";

type SiteHeaderProps = {
  isAdmin: boolean;
};

export default function SiteHeader({ isAdmin }: SiteHeaderProps) {
  return (
    <header className="site-header">
      <Link
        className="site-brand"
        href="/"
        aria-label="Историко-генеалогический портал Дмитрия Воробьева — на главную"
      >
        <Image
          className="site-brand__mark"
          src="/logo-gold.png"
          alt=""
          width={52}
          height={52}
          preload
        />
        <span className="site-brand__name">
          Историко-генеалогический портал Дмитрия Воробьева
        </span>
      </Link>
      <nav className="site-nav" aria-label="Основная навигация">
        <Link href="/o-proekte">О проекте</Link>
        {isAdmin ? <Link href="/nastroyki">Настройки</Link> : null}
        {isAdmin ? <Link href="/metriki">Метрики</Link> : null}
        <AdminAccess initialIsAdmin={isAdmin} />
      </nav>
    </header>
  );
}
