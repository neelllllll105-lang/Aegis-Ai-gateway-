import type { Metadata } from "next";
import DashboardShell from "./shell";

/**
 * Server component wrapper around the client dashboard shell.
 *
 * It exists purely so this route group can export `metadata`: a client component cannot,
 * and without it every authenticated page inherited the marketing site's title and its
 * indexable robots directive.
 */
export const metadata: Metadata = {
  title: {
    default: "Dashboard",
    template: "%s · Aegis",
  },
  // Everything under this group is behind authentication. A crawler that reaches one of
  // these URLs should not index it, and should not follow links out of it either.
  robots: { index: false, follow: false, nocache: true },
};

export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return <DashboardShell>{children}</DashboardShell>;
}
