import type { Metadata, Viewport } from "next";
import "./globals.css";

/**
 * Root metadata.
 *
 * Uses the Next.js metadata API rather than hand-written `<head>` tags, so the framework
 * deduplicates and merges correctly with per-route overrides.
 */
export const metadata: Metadata = {
  metadataBase: new URL(process.env.NEXT_PUBLIC_SITE_URL ?? "https://aegis.dev"),
  title: {
    default: "Aegis — Cut your AI bill by up to 90%",
    template: "%s · Aegis",
  },
  description:
    "An AI gateway that routes every request to the cheapest model that preserves quality, caches aggressively, and proves the savings per request. Change one base URL.",
  keywords: [
    "AI gateway",
    "LLM cost optimization",
    "OpenAI proxy",
    "Anthropic proxy",
    "AI cost reduction",
    "semantic cache",
    "model routing",
  ],
  authors: [{ name: "Aegis" }],
  openGraph: {
    type: "website",
    siteName: "Aegis",
    title: "Cut your AI bill by up to 90%. Keep the quality. Prove it.",
    description:
      "An AI gateway that routes every request to the cheapest model that preserves quality — with per-request, auditable savings attribution.",
  },
  twitter: {
    card: "summary_large_image",
    title: "Cut your AI bill by up to 90%. Keep the quality. Prove it.",
    description:
      "An AI gateway with quality-aware routing, semantic caching, and auditable savings.",
  },
  robots: {
    index: true,
    follow: true,
  },
};

export const viewport: Viewport = {
  themeColor: "#08090b",
  colorScheme: "dark",
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body className="min-h-screen antialiased">{children}</body>
    </html>
  );
}
