import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Usage & Tokens",
  description: "Token volume, cache hit rate and model mix over time.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
