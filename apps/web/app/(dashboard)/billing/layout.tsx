import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Billing",
  description: "Plan, credits and chargeback reporting.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
