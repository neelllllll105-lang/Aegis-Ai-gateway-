import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "People & Teams",
  description: "Members, roles and cost centers.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
