import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "API Keys",
  description: "Create, scope and revoke keys.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
