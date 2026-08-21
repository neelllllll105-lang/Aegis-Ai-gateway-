import Link from "next/link";
import { AegisLogo } from "@/components/ui";

/**
 * Aegis Marketing Shell — Pure Neutral Palette (#F9F8F6, #EFE9E3, #D9CFC7, #C9B59C).
 */
export default function MarketingLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="mx-auto max-w-7xl px-4 py-6 sm:px-6 sm:py-10 text-black">
      {/* Header with Aegis Sleek Minimalist Logo */}
      <header className="mb-8 border-b border-[#D9CFC7] pb-4">
        <div className="relative flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
          <Link
            href="/"
            className="flex items-center gap-2.5 text-lg font-black tracking-tight text-black group"
          >
            <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-[#EFE9E3] border border-[#D9CFC7] text-black transition-transform group-hover:scale-105 shadow-2xs">
              <AegisLogo className="w-5 h-5 text-black" />
            </div>
            <span className="text-black font-black text-xl tracking-tight">Aegis</span>
            <span className="hidden sm:inline-block text-[10px] uppercase font-black tracking-wider text-black bg-[#EFE9E3] px-2.5 py-0.5 rounded-full border border-[#D9CFC7]">
              AI Gateway
            </span>
          </Link>

          <div className="flex items-center gap-x-4 gap-y-2 text-sm sm:gap-x-6">
            <div className="hidden items-center gap-x-6 md:flex font-bold text-xs uppercase tracking-wide">
              <Link
                className="text-[#403B35] hover:text-black transition-colors"
                href="/connect"
              >
                How to use
              </Link>
              <Link
                className="text-[#403B35] hover:text-black transition-colors"
                href="/pricing"
              >
                Pricing
              </Link>
              <Link
                className="text-[#403B35] hover:text-black transition-colors"
                href="/faq"
              >
                FAQ
              </Link>
              <Link
                className="text-[#403B35] hover:text-black transition-colors"
                href="/docs"
              >
                Docs
              </Link>
              <Link
                className="text-[#403B35] hover:text-black transition-colors"
                href="/login"
              >
                Sign in
              </Link>
            </div>

            <Link
              href="/signup"
              className="rounded-lg bg-[#C9B59C] px-4 py-2 text-xs font-bold text-[#0A0A0A] transition-all hover:bg-[#BFAF98] shadow-xs active:scale-95 border border-[#BFAF98]"
            >
              Request access →
            </Link>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main>{children}</main>

      {/* Footer using the 4th Swatch #C9B59C (Warm Camel / Light Taupe) */}
      <footer className="mt-16 flex flex-wrap items-center justify-between gap-4 border-t border-[#D9CFC7] bg-[#C9B59C] rounded-2xl p-6 text-xs text-[#2A241E] font-bold shadow-xs">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <AegisLogo className="w-4 h-4 text-black" />
            <span className="font-black text-black">Aegis</span>
          </div>
          <span className="text-[#8F7D67]">·</span>
          <span>The control plane for AI cost and governance</span>
        </div>
        <div className="flex items-center gap-5">
          <Link className="hover:text-black transition-colors" href="/connect">
            How to use
          </Link>
          <Link className="hover:text-black transition-colors" href="/pricing">
            Pricing
          </Link>
          <Link className="hover:text-black transition-colors" href="/faq">
            FAQ
          </Link>
          <Link className="hover:text-black transition-colors" href="/docs">
            API Docs
          </Link>
        </div>
      </footer>
    </div>
  );
}
