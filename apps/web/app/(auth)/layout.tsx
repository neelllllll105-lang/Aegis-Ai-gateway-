import Link from "next/link";
import { AegisLogo } from "@/components/ui";

/** Auth shell: clean centered card on warm cream background with Aegis logo. */
export default function AuthLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center px-6 py-12 bg-[#FAF8F5]">
      <Link href="/" className="mb-8 flex items-center gap-2.5 group">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-[#FAF0E8] border border-[#E8BF9E] text-[#B85D26] transition-transform group-hover:scale-105 shadow-2xs">
          <AegisLogo className="w-5 h-5 text-[#B85D26]" />
        </div>
        <span className="text-xl font-black tracking-tight text-black">Aegis</span>
      </Link>

      <div className="w-full max-w-sm">{children}</div>
    </div>
  );
}
