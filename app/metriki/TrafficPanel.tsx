"use client";

import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import styles from "./TrafficPanel.module.css";

type Totals = { visitors: number; visits: number; pageviews: number };
type Day = Totals & { date: string };
type Breakdown = { name: string; visitors: number; visits: number };
type Data = {
  overview: Totals;
  daily: { rows: Day[] };
  pages: { rows: { url: string; pageviews: number }[] };
  sources: { rows: Breakdown[] };
  devices: { rows: Breakdown[] };
};
type Report = keyof Data;
type Period = "today" | "7d" | "30d" | "custom";
type Range = { period: Period; from?: string; to?: string };
type Meta = {
  period: { from: string; to: string; timezone: string };
  updatedAt: string;
  cacheTtlSeconds: number;
  sampled: boolean;
  sampleShare: number;
  dataLagSeconds: number | null;
};
type Envelope<K extends Report> = Meta & { report: K; data: Data[K] };
type Result<T> = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready"; value: T };
type Results = { [K in Report]: Result<Envelope<K>> };
const REPORTS: Report[] = ["overview", "daily", "pages", "sources", "devices"];
const number = new Intl.NumberFormat("ru-RU", { maximumFractionDigits: 0 });
const date = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
const updated = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", timeZone: "Europe/Moscow" });
const moscowDate = new Intl.DateTimeFormat("sv-SE", { year: "numeric", month: "2-digit", day: "2-digit", timeZone: "Europe/Moscow" });

function loading(): Results {
  return { overview: { kind: "loading" }, daily: { kind: "loading" }, pages: { kind: "loading" }, sources: { kind: "loading" }, devices: { kind: "loading" } };
}
function todayMoscow() {
  return moscowDate.format(new Date());
}
function validDate(value: string) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const parsed = new Date(`${value}T00:00:00Z`);
  return Number.isFinite(parsed.getTime()) && parsed.toISOString().slice(0, 10) === value;
}
function dayLabel(value: string) { return date.format(new Date(`${value}T00:00:00Z`)); }
function safePageUrl(value: string) {
  try { const url = new URL(value); return url.protocol === "http:" || url.protocol === "https:" ? value : undefined; }
  catch { return undefined; }
}
function Freshness({ report }: { report: Meta }) {
  return <p className={styles.freshness}>
    Обновлено: <time dateTime={report.updatedAt}>{updated.format(new Date(report.updatedAt))}</time> МСК.
    {report.sampled ? ` Данные выборочные (${number.format(report.sampleShare * 100)}% визитов).` : ""}
    {report.dataLagSeconds !== null && report.dataLagSeconds > 0 ? ` Задержка данных Яндекса: около ${Math.ceil(report.dataLagSeconds / 60)} мин.` : ""}
  </p>;
}
function ReportBlock<T extends Meta>({ id, title, state, retry, children }: {
  id: string; title: string; state: Result<T>; retry: () => void; children: (value: T) => ReactNode;
}) {
  return <section className={styles.report} aria-labelledby={id} aria-busy={state.kind === "loading"}>
    <h3 id={id}>{title}</h3>
    {state.kind === "loading" ? <p className={styles.note} role="status">Загрузка отчёта…</p> :
      state.kind === "error" ? <div className={styles.error} role="alert"><p>{state.message}</p><button type="button" onClick={retry}>Повторить</button></div> :
        <>{children(state.value)}<Freshness report={state.value} /></>}
  </section>;
}
function BreakdownTable({ rows }: { rows: Breakdown[] }) {
  if (rows.length === 0) return <p className={styles.note}>За выбранный период данных нет.</p>;
  return <div className={styles.tableWrap}><table><thead><tr><th scope="col">Название</th><th scope="col">Посетители</th><th scope="col">Визиты</th></tr></thead>
    <tbody>{rows.map((row, index) => <tr key={`${row.name}-${index}`}><th scope="row">{row.name}</th><td>{number.format(row.visitors)}</td><td>{number.format(row.visits)}</td></tr>)}</tbody></table></div>;
}
function DailyChart({ rows }: { rows: Day[] }) {
  const [showTable, setShowTable] = useState(false);
  const peak = rows.reduce((max, row) => Math.max(max, row.visits), 0);
  const x = (index: number) => rows.length === 1 ? 350 : 48 + index / (rows.length - 1) * 628;
  const y = (value: number) => 190 - value / Math.max(peak, 1) * 162;
  const points = rows.map((row, index) => `${x(index)},${y(row.visits)}`).join(" ");
  return <>
    {peak === 0 ? <p className={styles.note}>В выбранном периоде визитов нет.</p> : <>
      <svg className={styles.chart} viewBox="0 0 700 225" role="img" aria-label={`Визиты по дням: ${dayLabel(rows[0].date)} — ${dayLabel(rows[rows.length - 1].date)}. Максимум ${number.format(peak)} визитов за день. Точные значения доступны в таблице ниже.`}>
        {[0, 0.5, 1].map(fraction => <g key={fraction}><line x1="48" x2="676" y1={y(peak * fraction)} y2={y(peak * fraction)} className={styles.gridLine} /><text x="40" y={y(peak * fraction) + 4} textAnchor="end">{number.format(peak * fraction)}</text></g>)}
        <polyline points={points} className={styles.graphLine} />
        {rows.length === 1 ? <circle cx={x(0)} cy={y(rows[0].visits)} r="4" className={styles.graphPoint} /> : null}
        <text x="48" y="216">{dayLabel(rows[0].date)}</text>
        {rows.length > 1 ? <text x="676" y="216" textAnchor="end">{dayLabel(rows[rows.length - 1].date)}</text> : null}
      </svg>
      <p className={styles.note}>График показывает визиты. Уникальные посетители за весь период рассчитываются отдельно, а не суммируются по дням.</p>
    </>}
    <details className={styles.dailyTable} onToggle={event => setShowTable(event.currentTarget.open)}>
      <summary>Показать данные по дням</summary>
      {showTable ? <div className={styles.tableWrap}><table><thead><tr><th scope="col">Дата</th><th scope="col">Посетители</th><th scope="col">Визиты</th><th scope="col">Просмотры</th></tr></thead><tbody>
        {rows.map(row => <tr key={row.date}><th scope="row">{dayLabel(row.date)}</th><td>{number.format(row.visitors)}</td><td>{number.format(row.visits)}</td><td>{number.format(row.pageviews)}</td></tr>)}
      </tbody></table></div> : null}
    </details>
  </>;
}

export default function TrafficPanel() {
  const [selection, setSelection] = useState<Period>("7d");
  const [range, setRange] = useState<Range>({ period: "7d" });
  const [from, setFrom] = useState(todayMoscow);
  const [to, setTo] = useState(todayMoscow);
  const [dateError, setDateError] = useState("");
  const [results, setResults] = useState<Results>(loading);
  const [revision, setRevision] = useState(0);
  const query = new URLSearchParams({ period: range.period, ...(range.period === "custom" ? { from: range.from!, to: range.to! } : {}) }).toString();
  const retry = () => setRevision(value => value + 1);
  const overviewError = results.overview.kind === "error" ? results.overview.message : null;
  const commonError = overviewError && REPORTS.every(name => {
    const result = results[name];
    return result.kind === "error" && result.message === overviewError;
  }) ? overviewError : null;
  const today = todayMoscow();

  useEffect(() => {
    const controller = new AbortController();
    let active = true;
    setResults(loading());
    async function load<K extends Report>(report: K) {
      try {
        const response = await fetch(`/api/metrika/traffic?${query}&report=${report}`, { signal: controller.signal, cache: "no-store" });
        const body: unknown = await response.json();
        if (!response.ok) {
          const message = response.status === 401 ? "Сессия администратора истекла. Войдите снова." :
            typeof body === "object" && body !== null && "error" in body && typeof body.error === "string" ? body.error : "Не удалось получить отчёт Яндекс Метрики.";
          throw new Error(message);
        }
        const value = body as Envelope<K>;
        if (value.report !== report || !value.data || !value.period || typeof value.updatedAt !== "string") throw new Error("Сервер вернул некорректный отчёт.");
        if (active) setResults(previous => ({ ...previous, [report]: { kind: "ready", value } }));
      } catch (error) {
        if (!active || controller.signal.aborted) return;
        const message = error instanceof Error ? error.message : "Яндекс Метрика недоступна. Повторите попытку позже.";
        setResults(previous => ({ ...previous, [report]: { kind: "error", message } }));
      }
    }
    REPORTS.forEach(report => { void load(report); });
    return () => { active = false; controller.abort(); };
  }, [query, revision]);

  function applyCustom(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!validDate(from) || !validDate(to)) { setDateError("Укажите корректные даты начала и окончания."); return; }
    if (from > to) { setDateError("Дата начала должна быть не позже даты окончания."); return; }
    if (to > todayMoscow()) { setDateError("Нельзя выбрать будущие даты."); return; }
    setDateError(""); setRange({ period: "custom", from, to });
  }

  return <section className={styles.panel} aria-labelledby="traffic-heading">
    <div className={styles.heading}><h2 id="traffic-heading">Посещаемость</h2><button type="button" onClick={retry}>Обновить данные</button></div>
    <p className={styles.note}>Данные Яндекс Метрики. Отчёты кэшируются на сервере на 10 минут и не обновляются в реальном времени. Даты и время — московские.</p>
    <div className={styles.controls}>
      <label className={styles.field}>Период<select value={selection} onChange={event => {
        const next = event.target.value as Period; setSelection(next); setDateError("");
        if (next !== "custom") setRange({ period: next });
      }}><option value="today">Сегодня</option><option value="7d">7 дней</option><option value="30d">30 дней</option><option value="custom">Произвольные даты</option></select></label>
      {selection === "custom" ? <form className={styles.customDates} onSubmit={applyCustom}>
        <label className={styles.field}>С даты<input type="date" required value={from} max={today} onChange={event => setFrom(event.target.value)} /></label>
        <label className={styles.field}>По дату<input type="date" required value={to} max={today} onChange={event => setTo(event.target.value)} /></label>
        <button type="submit">Показать</button>
      </form> : null}
    </div>
    {dateError ? <p className={styles.error} role="alert">{dateError}</p> : null}
    {commonError ? <div className={styles.error} role="alert"><p>{commonError}</p><button type="button" onClick={retry}>Повторить</button></div> : null}
    {!commonError ? <>
    {results.overview.kind === "ready" ? <p className={styles.period}>Отчёт за {dayLabel(results.overview.value.period.from)} — {dayLabel(results.overview.value.period.to)}</p> : null}
    <ReportBlock id="traffic-overview" title="Сводка за период" state={results.overview} retry={retry}>{report => <>
      <dl className={styles.cards}>
        <div><dt>Посетители</dt><dd>{number.format(report.data.visitors)}<span className={styles.cardNote}>Уникальные за весь период</span></dd></div>
        <div><dt>Визиты</dt><dd>{number.format(report.data.visits)}</dd></div>
        <div><dt>Просмотры</dt><dd>{number.format(report.data.pageviews)}</dd></div>
      </dl>
      {report.data.visits === 0 && report.data.pageviews === 0 ? <p className={styles.note}>За выбранный период посещений нет.</p> : null}
    </>}</ReportBlock>
    <ReportBlock id="traffic-daily" title="Визиты по дням" state={results.daily} retry={retry}>{report => <DailyChart key={`${report.period.from}-${report.period.to}`} rows={report.data.rows} />}</ReportBlock>
    <ReportBlock id="traffic-pages" title="Популярные страницы" state={results.pages} retry={retry}>{report => report.data.rows.length === 0 ? <p className={styles.note}>За выбранный период просмотров нет.</p> :
      <div className={styles.tableWrap}><table><caption>20 самых просматриваемых страниц</caption><thead><tr><th scope="col">Страница</th><th scope="col">Просмотры</th></tr></thead><tbody>
        {report.data.rows.map((row, index) => {
          const href = safePageUrl(row.url);
          return <tr key={`${row.url}-${index}`}><th scope="row">{href ? <a href={href} target="_blank" rel="noopener noreferrer">{row.url}</a> : row.url}</th><td>{number.format(row.pageviews)}</td></tr>;
        })}
      </tbody></table></div>}
    </ReportBlock>
    <ReportBlock id="traffic-sources" title="Источники переходов" state={results.sources} retry={retry}>{report => <BreakdownTable rows={report.data.rows} />}</ReportBlock>
    <ReportBlock id="traffic-devices" title="Устройства посетителей" state={results.devices} retry={retry}>{report => <BreakdownTable rows={report.data.rows} />}</ReportBlock>
    </> : null}
  </section>;
}
