import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Overview",
  description: "Savings, spend and request volume at a glance.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
