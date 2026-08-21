import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Settings",
  description: "Organisation and data-retention settings.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
