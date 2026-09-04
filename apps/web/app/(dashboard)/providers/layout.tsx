import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Providers",
  description: "Bring-your-own provider credentials.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
