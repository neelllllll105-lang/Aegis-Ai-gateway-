import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Savings & ROI",
  description: "What Aegis saved, how, and what it charged for it.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
