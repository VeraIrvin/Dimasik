import type { Metadata } from "next";
import type { ReactNode } from "react";
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
      <body>{children}</body>
    </html>
  );
}
