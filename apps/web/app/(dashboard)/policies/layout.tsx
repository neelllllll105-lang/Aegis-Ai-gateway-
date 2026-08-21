import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Policies",
  description: "Routing rules that override the cost router.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
