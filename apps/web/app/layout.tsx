import type { Metadata, Viewport } from "next";
import { Space_Grotesk, Spectral, Caveat, JetBrains_Mono } from "next/font/google";
import "./globals.css";

// Four voices: UI chrome (Space Grotesk), documents & headings (Spectral),
// exact/machine values (JetBrains Mono), and the human voice (Caveat) —
// used sparingly, never as a system font.
const sans = Space_Grotesk({
  subsets: ["latin"],
  variable: "--font-sans",
  display: "swap",
});

const serif = Spectral({
  subsets: ["latin"],
  weight: ["400", "500", "600", "700"],
  style: ["normal", "italic"],
  variable: "--font-serif",
  display: "swap",
});

const hand = Caveat({
  subsets: ["latin"],
  weight: ["500", "600", "700"],
  variable: "--font-hand",
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
    "The intelligent routing gateway for US enterprises. Reduce LLM API spend by up to 70% with quality-aware routing, semantic caching, and auditable per-request receipts. Change one base URL.",
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
    title: "Cut your Enterprise AI bill by up to 70%. Keep full quality. Prove it.",
    description:
      "Enterprise AI gateway with quality-aware routing, semantic caching, and real-time auditable savings attribution.",
  },
  twitter: {
    card: "summary_large_image",
    title: "Aegis — Cut Enterprise AI bills by up to 70%",
    description:
      "Intelligent AI gateway with quality-aware routing, semantic caching, and auditable savings.",
  },
  robots: {
    index: true,
    follow: true,
  },
  icons: {
    icon: "/favicon.svg",
    shortcut: "/favicon.svg",
    apple: "/favicon.svg",
  },
};

export const viewport: Viewport = {
  themeColor: "#EAE3D3",
  colorScheme: "light",
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={`${sans.variable} ${serif.variable} ${hand.variable} ${mono.variable}`}>
      <body className="min-h-screen bg-[var(--color-bg)] text-[var(--color-paper-on-desk)] font-sans antialiased selection:bg-[var(--color-accent-bg)] selection:text-[var(--color-accent-dark)]">
        {children}
      </body>
    </html>
  );
}
