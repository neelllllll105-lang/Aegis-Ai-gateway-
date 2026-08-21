import type { Metadata, Viewport } from "next";
import { Plus_Jakarta_Sans, JetBrains_Mono } from "next/font/google";
import "./globals.css";

const sans = Plus_Jakarta_Sans({
  subsets: ["latin"],
  variable: "--font-sans",
  display: "swap",
});

const mono = JetBrains_Mono({
  subsets: ["latin"],
  variable: "--font-mono",
  display: "swap",
});

export const metadata: Metadata = {
  metadataBase: new URL(process.env.NEXT_PUBLIC_SITE_URL ?? "https://aegis.dev"),
  title: {
    default: "Aegis — Enterprise AI Cost Optimization & Intelligent Gateway",
    template: "%s · Aegis",
  },
  description:
    "The intelligent routing gateway for US enterprises. Reduce LLM API spend by up to 90% with quality-aware routing, semantic caching, and auditable per-request receipts. Change one base URL.",
  keywords: [
    "AI gateway",
    "LLM cost optimization",
    "Enterprise AI spend",
    "OpenAI proxy",
    "Anthropic proxy",
    "AI cost reduction",
    "semantic cache",
    "model routing",
    "Y Combinator AI startup",
  ],
  authors: [{ name: "Aegis" }],
  openGraph: {
    type: "website",
    siteName: "Aegis",
    title: "Cut your Enterprise AI bill by up to 90%. Keep full quality. Prove it.",
    description:
      "Enterprise AI gateway with quality-aware routing, semantic caching, and real-time auditable savings attribution.",
  },
  twitter: {
    card: "summary_large_image",
    title: "Aegis — Cut Enterprise AI bills by up to 90%",
    description:
      "Intelligent AI gateway with quality-aware routing, semantic caching, and auditable savings.",
  },
  robots: {
    index: true,
    follow: true,
  },
};

export const viewport: Viewport = {
  themeColor: "#F9F8F6",
  colorScheme: "light",
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={`${sans.variable} ${mono.variable}`}>
      <body className="min-h-screen bg-[var(--color-base)] text-[var(--color-ink)] font-sans antialiased selection:bg-[var(--color-accent-wash)] selection:text-[var(--color-accent-dark)]">
        {children}
      </body>
    </html>
  );
}
