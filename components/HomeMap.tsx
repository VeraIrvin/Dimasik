'use client';

import { useEffect, useId, useRef, useState } from 'react';
import type { PublishedOption, Settlement } from '@/lib/gubernia-publications';
import type { SiteSettings } from '@/lib/site-settings';
import GuberniasMap from './GuberniasMap';
import SiteHeader from './SiteHeader';

type HomeView = 'gubernias' | 'settlements';

type HomeMapProps = {
  isAdmin: boolean;
  settlements: Settlement[];
  provinces: PublishedOption[];
  settings: SiteSettings;
  initialView: HomeView;
};

export default function HomeMap({
  isAdmin,
  settlements,
  provinces,
  settings,
  initialView,
}: HomeMapProps) {
  const [searchVisible, setSearchVisible] = useState(false);
  const searchRegionId = useId();
  const rootRef = useRef<HTMLElement>(null);

  useEffect(() => {
    if (!searchVisible) return;
    rootRef.current
      ?.querySelector<HTMLInputElement>(`#${CSS.escape(searchRegionId)} input`)
      ?.focus();
  }, [searchRegionId, searchVisible]);

  const searchLabel = searchVisible
    ? 'Скрыть поиск по записям'
    : 'Показать поиск по записям';

  return (
    <main ref={rootRef} className="site-shell">
      <SiteHeader
        isAdmin={isAdmin}
        searchControl={
          <button
            className="site-nav__search-button"
            type="button"
            aria-label={searchLabel}
            title={searchLabel}
            aria-expanded={searchVisible}
            aria-controls={searchRegionId}
            onClick={() => setSearchVisible((visible) => !visible)}
          >
            <svg
              aria-hidden="true"
              focusable="false"
              viewBox="0 0 24 24"
              width="20"
              height="20"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
            >
              <circle cx="10.5" cy="10.5" r="6.5" />
              <path d="m15.4 15.4 4.1 4.1" />
            </svg>
          </button>
        }
      />
      <GuberniasMap
        isAdmin={isAdmin}
        settings={settings}
        settlements={settlements}
        provinces={provinces}
        initialView={initialView}
        searchVisible={searchVisible}
        searchRegionId={searchRegionId}
      />
    </main>
  );
}
