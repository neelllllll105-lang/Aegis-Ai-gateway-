import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Models",
  description: "The model catalogue and what each one costs.",
};

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
