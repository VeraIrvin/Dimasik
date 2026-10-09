import type { Metadata } from "next";
import { Suspense, type ReactNode } from "react";
import YandexMetrika from "@/components/YandexMetrika";
import "maplibre-gl/dist/maplibre-gl.css";
import "./globals.css";

export const metadata: Metadata = {
  title: "Дмитрий Воробьев",
  description: "Историко-генеалогический портал Дмитрия Воробьева. Историческая карта 76 губерний Российской империи по реконструкции границ 1897 года.",
  icons: { icon: "/favicon-dark.png?v=3ab8696a" },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="ru">
      <body>
        {children}
        <Suspense fallback={null}>
          <YandexMetrika />
        </Suspense>
        <noscript>
          <div>
            <img
              src="https://mc.yandex.ru/watch/113584085"
              style={{ position: "absolute", left: "-9999px" }}
              alt=""
            />
          </div>
        </noscript>
      </body>
    </html>
  );
}
