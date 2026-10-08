import Link from "next/link";

export default function DocumentsPage() {
  return (
    <main className="content-page">
      <p className="content-page__eyebrow">Архив</p>
      <h1 className="content-page__title">Документы</h1>
      <p className="content-page__text">
        Архивные документы пока не добавлены. Раздел будет наполнен после проверки и описания источников.
      </p>
      <Link className="content-page__back" href="/">
        ← Вернуться к карте
      </Link>
    </main>
  );
}
