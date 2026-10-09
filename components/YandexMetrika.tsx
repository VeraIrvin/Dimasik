"use client";

import Script from "next/script";
import { usePathname, useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";

const COUNTER_ID = 113584085;

type MetrikaWindow = Window & {
  ym?: (counterId: typeof COUNTER_ID, method: "hit", url: string) => void;
};

const bootstrap = `
(function(m,e,t,r,i,k,a){
    m[i]=m[i]||function(){(m[i].a=m[i].a||[]).push(arguments)};
    m[i].l=1*new Date();
    for (var j = 0; j < document.scripts.length; j++) {
        if (document.scripts[j].src === r) { return; }
    }
    k=e.createElement(t),a=e.getElementsByTagName(t)[0],k.async=1,k.src=r,a.parentNode.insertBefore(k,a)
})(window, document, "script", "https://mc.yandex.ru/metrika/tag.js?id=113584085", "ym");

ym(113584085, "init", {
    ssr: true,
    webvisor: true,
    clickmap: true,
    ecommerce: "dataLayer",
    referrer: document.referrer,
    url: location.href,
    accurateTrackBounce: true,
    trackLinks: true,
    defer: true
});
`;

export default function YandexMetrika() {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const queryString = searchParams.toString();
  const [ready, setReady] = useState(false);
  const lastHref = useRef<string | null>(null);

  useEffect(() => {
    if (!ready) return;

    const metrika = (window as MetrikaWindow).ym;
    const href = window.location.href;
    if (!metrika || lastHref.current === href) return;

    metrika(COUNTER_ID, "hit", href);
    lastHref.current = href;
  }, [ready, pathname, queryString]);

  return (
    <Script
      id="yandex-metrika-113584085"
      strategy="afterInteractive"
      onReady={() => setReady(true)}
    >
      {bootstrap}
    </Script>
  );
}
