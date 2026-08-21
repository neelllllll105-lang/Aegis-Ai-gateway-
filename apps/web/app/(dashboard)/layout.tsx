"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api, ApiError, type Organization } from "@/lib/api";
import { AegisLogo } from "@/components/ui";

const NAV = [
  { href: "/dashboard", label: "Overview", icon: "M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-6 0a1 1 0 001-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 001 1m-6 0h6" },
  { href: "/savings", label: "Savings & ROI", icon: "M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1M21 12a9 9 0 11-18 0 9 9 0 0118 0z" },
  { href: "/usage", label: "Usage & Tokens", icon: "M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" },
  { href: "/requests", label: "Request Logs", icon: "M4 6h16M4 10h16M4 14h16M4 18h16" },
  { href: "/keys", label: "API Keys", icon: "M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" },
  { href: "/providers", label: "Providers", icon: "M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" },
  { href: "/settings", label: "Settings", icon: "M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" },
];

export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const [org, setOrg] = useState<Organization | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "error">("loading");
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    api
      .me()
      .then((response) => {
        if (cancelled) return;
        setOrg(response.organization);
        setState("ready");
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        if (caught instanceof ApiError && caught.isUnauthorized) {
          router.replace("/login");
          return;
        }
        setMessage(
          caught instanceof ApiError
            ? caught.message
            : "Could not reach the Aegis API. Is the gateway running on port 8080?",
        );
        setState("error");
      });

    return () => {
      cancelled = true;
    };
  }, [router]);

  async function handleSignOut() {
    try {
      await api.logout();
    } finally {
      router.push("/login");
    }
  }

  if (state === "loading") {
    return (
      <div className="flex min-h-screen items-center justify-center bg-[#FAF8F5]">
        <div className="flex items-center gap-2 text-xs font-bold text-black">
          <span className="flex h-2 w-2 rounded-full bg-[#B85D26] animate-pulse" />
          Loading dashboard session…
        </div>
      </div>
    );
  }

  if (state === "error") {
    return (
      <div className="flex min-h-screen items-center justify-center px-6 bg-[#FAF8F5]">
        <div className="rounded-2xl border border-[#D8CFC4] bg-white max-w-md p-6 shadow-xs">
          <div className="flex items-center gap-2 text-[#D97706] mb-2">
            <svg className="w-5 h-5" viewBox="0 0 20 20" fill="currentColor">
              <path fillRule="evenodd" d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z" clipRule="evenodd" />
            </svg>
            <h1 className="text-base font-black text-black">
              API Connection Notice
            </h1>
          </div>
          <p className="text-xs text-[#3D362F] leading-relaxed font-medium">{message}</p>
          <div className="mt-4 rounded-xl bg-[#FAF8F5] p-3 border border-[#D8CFC4]">
            <p className="text-[11px] text-black font-mono font-bold">
              cd apps/gateway &amp;&amp; cargo run --bin aegis-gateway
            </p>
          </div>
          <div className="mt-4 flex justify-end">
            <button
              onClick={() => window.location.reload()}
              className="rounded-xl bg-[#B85D26] px-4 py-2 text-xs font-black text-white shadow-xs hover:bg-[#96481A]"
            >
              Retry Connection
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-screen bg-[#FAF8F5] text-black">
      {/* Sidebar */}
      <aside className="hidden w-60 shrink-0 border-r border-[#D8CFC4] bg-[#EFE8DF] md:block">
        <div className="sticky top-0 flex h-screen flex-col p-4">
          <Link href="/dashboard" className="flex items-center gap-2.5 px-2 py-2 group">
            <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-[#FAF0E8] border border-[#E8BF9E] text-[#B85D26] shadow-2xs">
              <AegisLogo className="w-5 h-5 text-[#B85D26]" />
            </div>
            <span className="text-sm font-black tracking-tight text-black">
              Aegis Dashboard
            </span>
          </Link>

          <nav className="mt-6 flex-1 space-y-1" aria-label="Dashboard">
            {NAV.map((item) => {
              const active = pathname === item.href;
              return (
                <Link
                  key={item.href}
                  href={item.href}
                  aria-current={active ? "page" : undefined}
                  className={`flex items-center gap-2.5 rounded-xl px-3 py-2 text-xs font-bold transition-all ${
                    active
                      ? "bg-white text-black border border-[#D8CFC4] shadow-xs"
                      : "text-[#3D362F] hover:bg-white/60 hover:text-black"
                  }`}
                >
                  <svg className="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d={item.icon} />
                  </svg>
                  <span>{item.label}</span>
                </Link>
              );
            })}
          </nav>

          <div className="border-t border-[#D8CFC4] pt-3">
            {org && (
              <div className="rounded-xl bg-white p-2.5 mb-2 border border-[#D8CFC4]">
                <div className="truncate text-xs font-black text-black">
                  {org.name}
                </div>
                <div className="text-[10px] capitalize text-[#B85D26] font-black">
                  {org.plan} tier
                </div>
              </div>
            )}
            <button
              type="button"
              onClick={handleSignOut}
              className="w-full rounded-xl px-2.5 py-1.5 text-left text-xs font-bold text-[#3D362F] transition-colors hover:bg-white hover:text-black"
            >
              Sign out
            </button>
          </div>
        </div>
      </aside>

      {/* Main Content Area */}
      <div className="min-w-0 flex-1">
        {/* Mobile Header Nav */}
        <nav
          className="flex gap-1 overflow-x-auto border-b border-[#D8CFC4] bg-[#EFE8DF] px-4 py-2.5 md:hidden"
          aria-label="Dashboard"
        >
          {NAV.map((item) => (
            <Link
              key={item.href}
              href={item.href}
              aria-current={pathname === item.href ? "page" : undefined}
              className={`whitespace-nowrap rounded-xl px-3 py-1.5 text-xs font-bold ${
                pathname === item.href
                  ? "bg-white text-black border border-[#D8CFC4]"
                  : "text-[#3D362F] hover:bg-white/60 hover:text-black"
              }`}
            >
              {item.label}
            </Link>
          ))}
        </nav>

        <main className="px-4 py-8 sm:px-8">
          <div className="mx-auto max-w-5xl">{children}</div>
        </main>
      </div>
    </div>
  );
}
