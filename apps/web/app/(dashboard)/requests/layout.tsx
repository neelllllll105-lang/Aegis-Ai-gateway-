import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Request Logs",
  description: "Per-request routing decisions and costs.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
