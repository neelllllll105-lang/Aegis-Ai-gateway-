import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Budgets",
  description: "Spend limits and anomaly alerts.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
