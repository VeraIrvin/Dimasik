'use client';

import Link from 'next/link';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import type { FocusEvent as ReactFocusEvent, KeyboardEvent as ReactKeyboardEvent } from 'react';
import styles from './MapPostSearch.module.css';

const SEARCH_URL = '/api/posts/search';
const QUERY_LIMIT = 200;
const DEBOUNCE_MS = 250;

type HomeView = 'gubernias' | 'settlements';

type SearchResult = {
  id: string;
  title: string;
  guberniaId: string;
  guberniaName: string;
  settlementId: string | null;
  settlementName: string | null;
  href: string;
};

type SearchResponse = {
  query: string;
  results: SearchResult[];
  total: number;
  offset: number;
  nextOffset: number | null;
};

type MapPostSearchProps = {
  view: HomeView;
};

type RequestState = 'idle' | 'loading' | 'ready';

function normalizeQuery(value: string): string {
  return value.trim().replace(/\s+/gu, ' ');
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function isSafePostHref(value: string): boolean {
  return (
    (value.startsWith('/guberniya/') ||
      value.startsWith('/naselennyy-punkt/') ||
      value.startsWith('/uezd/')) &&
    !value.startsWith('//') &&
    /^\/[^?#]+#post-[^?#]+$/u.test(value)
  );
}

function parseSearchResponse(value: unknown): SearchResponse {
  const payload = value as Partial<SearchResponse> | null;
  if (
    typeof payload !== 'object' ||
    payload === null ||
    typeof payload.query !== 'string' ||
    !Array.isArray(payload.results) ||
    !Number.isSafeInteger(payload.total) ||
    (payload.total as number) < 0 ||
    !Number.isSafeInteger(payload.offset) ||
    (payload.offset as number) < 0 ||
    !(
      payload.nextOffset === null ||
      (Number.isSafeInteger(payload.nextOffset) && (payload.nextOffset as number) >= 0)
    )
  ) {
    throw new Error('Сервис поиска вернул данные в неизвестном формате.');
  }

  const results = payload.results.map((candidate): SearchResult => {
    const result = candidate as Partial<SearchResult> | null;
    if (
      typeof result !== 'object' ||
      result === null ||
      typeof result.id !== 'string' ||
      result.id.length === 0 ||
      typeof result.title !== 'string' ||
      typeof result.guberniaId !== 'string' ||
      typeof result.guberniaName !== 'string' ||
      !isNullableString(result.settlementId) ||
      !isNullableString(result.settlementName) ||
      typeof result.href !== 'string' ||
      !isSafePostHref(result.href)
    ) {
      throw new Error('Сервис поиска вернул данные в неизвестном формате.');
    }
    return result as SearchResult;
  });

  const seenIds = new Set<string>();
  for (const result of results) {
    if (seenIds.has(result.id)) {
      throw new Error('Сервис поиска вернул повторяющиеся записи.');
    }
    seenIds.add(result.id);
  }
  if (
    results.length > 50 ||
    (payload.nextOffset !== null && (payload.nextOffset as number) <= (payload.offset as number))
  ) {
    throw new Error('Сервис поиска вернул неверную страницу результатов.');
  }

  return {
    query: payload.query,
    results,
    total: payload.total as number,
    offset: payload.offset as number,
    nextOffset: payload.nextOffset as number | null,
  };
}

function destinationHref(href: string, query: string, view: HomeView): string {
  const hashAt = href.indexOf('#');
  const path = href.slice(0, hashAt);
  const hash = href.slice(hashAt);
  const parameters = new URLSearchParams({ highlight: query });
  if (view === 'settlements') parameters.set('from', 'settlements');
  return `${path}?${parameters.toString()}${hash}`;
}
export default function MapPostSearch({ view }: MapPostSearchProps) {
  const resultsId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<AbortController | null>(null);
  const sequenceRef = useRef(0);
  const [input, setInput] = useState('');
  const [activeQuery, setActiveQuery] = useState('');
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [requestState, setRequestState] = useState<RequestState>('idle');
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState('');
  const [open, setOpen] = useState(false);

  const requestPage = useCallback(async (query: string, offset: number, append: boolean) => {
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    const sequence = ++sequenceRef.current;

    if (append) setLoadingMore(true);
    else setRequestState('loading');
    setError('');

    try {
      const parameters = new URLSearchParams({ q: query, offset: String(offset) });
      const fetchResponse = await fetch(`${SEARCH_URL}?${parameters.toString()}`, {
        cache: 'no-store',
        signal: controller.signal,
      });
      if (!fetchResponse.ok) {
        throw new Error(`HTTP ${fetchResponse.status}`);
      }
      const payload = parseSearchResponse(await fetchResponse.json());
      if (
        sequence !== sequenceRef.current ||
        normalizeQuery(payload.query).toLowerCase() !== query.toLowerCase() ||
        payload.offset !== offset
      ) {
        return;
      }

      if (append) {
        setResponse((current) => {
          if (
            !current ||
            current.nextOffset !== offset ||
            normalizeQuery(current.query).toLowerCase() !== query.toLowerCase()
          ) {
            return current;
          }
          const existingIds = new Set(current.results.map((result) => result.id));
          const additional = payload.results.filter((result) => !existingIds.has(result.id));
          return { ...payload, results: [...current.results, ...additional] };
        });
      } else {
        setResponse(payload);
        setRequestState('ready');
      }
    } catch {
      if (controller.signal.aborted || sequence !== sequenceRef.current) return;
      setError('Не удалось выполнить поиск. Попробуйте ещё раз.');
      if (!append) setRequestState('ready');
    } finally {
      if (sequence === sequenceRef.current) {
        setLoadingMore(false);
        if (!append) setRequestState('ready');
      }
    }
  }, []);

  useEffect(() => {
    const query = normalizeQuery(input);
    controllerRef.current?.abort();
    sequenceRef.current += 1;

    if (query.length === 0) {
      setActiveQuery('');
      setResponse(null);
      setRequestState('idle');
      setLoadingMore(false);
      setError('');
      setOpen(false);
      return;
    }

    setActiveQuery(query);
    setResponse(null);
    setRequestState('loading');
    setLoadingMore(false);
    setError('');
    setOpen(true);
    const timeout = window.setTimeout(() => void requestPage(query, 0, false), DEBOUNCE_MS);
    return () => window.clearTimeout(timeout);
  }, [input, requestPage]);

  useEffect(() => {
    const closeOutside = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', closeOutside);
    return () => document.removeEventListener('pointerdown', closeOutside);
  }, []);

  useEffect(() => () => controllerRef.current?.abort(), []);

  function handleBlur(event: ReactFocusEvent<HTMLDivElement>) {
    if (event.relatedTarget && !event.currentTarget.contains(event.relatedTarget)) setOpen(false);
  }

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    (rootRef.current?.querySelector('input') as HTMLInputElement | null)?.focus();
    setOpen(false);
  }

  function clearSearch() {
    setInput('');
    setOpen(false);
  }

  const showPopup = open && activeQuery.length > 0;
  const hasResults = response !== null && response.results.length > 0;

  return (
    <div
      ref={rootRef}
      className={styles.search}
      onBlur={handleBlur}
      onKeyDown={handleKeyDown}
    >
      <div className={styles.fieldGroup}>
        <label className={styles.label} htmlFor={`${resultsId}-input`}>
          Поиск по записям
        </label>
        <div className={styles.inputWrap}>
          <input
            id={`${resultsId}-input`}
            className={styles.input}
            type="search"
            value={input}
            maxLength={QUERY_LIMIT}
            autoComplete="off"
            placeholder="Поиск по записям"
            aria-controls={resultsId}
            aria-expanded={showPopup}
            onChange={(event) => setInput(event.target.value.slice(0, QUERY_LIMIT))}
            onFocus={() => {
              if (activeQuery.length > 0) setOpen(true);
            }}
            onClick={() => {
              if (activeQuery.length > 0) setOpen(true);
            }}
          />
          {input.length > 0 ? (
            <button
              className={styles.clearButton}
              type="button"
              onClick={clearSearch}
              aria-label="Очистить поиск"
            >
              <span aria-hidden="true">×</span>
            </button>
          ) : null}
        </div>

        {showPopup ? (
          <div id={resultsId} className={styles.popup} aria-live="polite">
            {requestState === 'loading' ? (
              <p className={styles.state}>Ищем записи…</p>
            ) : null}

            {error ? (
              <p className={styles.error} role="alert">
                {error}
              </p>
            ) : null}

            {requestState === 'ready' && response ? (
              <p className={styles.count}>Найдено: {response.total}</p>
            ) : null}

            {requestState === 'ready' && response && !hasResults && !error ? (
              <p className={styles.state}>По вашему запросу ничего не найдено.</p>
            ) : null}

            {hasResults && response ? (
              <>
                <ul className={styles.results}>
                  {response.results.map((result) => (
                    <li key={result.id}>
                      <Link
                        className={styles.resultLink}
                        href={destinationHref(result.href, activeQuery, view)}
                        target="_blank"
                        rel="noopener"
                        prefetch={false}
                      >
                        <span className={styles.resultTitle}>{result.title}</span>
                        <span className={styles.resultLocation}>
                          {result.settlementName
                            ? `${result.settlementName}, ${result.guberniaName}`
                            : result.guberniaName}
                        </span>
                      </Link>
                    </li>
                  ))}
                </ul>
                {response.nextOffset !== null ? (
                  <button
                    className={styles.moreButton}
                    type="button"
                    disabled={loadingMore}
                    onClick={() => void requestPage(activeQuery, response.nextOffset as number, true)}
                  >
                    {loadingMore ? 'Загружаем…' : 'Показать ещё'}
                  </button>
                ) : null}
              </>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}
