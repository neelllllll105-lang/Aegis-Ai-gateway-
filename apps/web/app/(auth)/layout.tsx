import Link from "next/link";
import { AegisLogo } from "@/components/ui";

/** Auth shell: a paper card floating on the dark desk, Aegis wordmark above it. */
export default function AuthLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center px-6 py-12 bg-[var(--color-bg)]">
      <Link href="/" className="mb-8 flex items-center gap-2.5 group">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--color-surface2)] border-[1.5px] border-[var(--color-ink)] text-[var(--color-ink)] transition-transform group-hover:scale-105 shadow-[2px_2px_0_var(--color-desk-line)]">
          <AegisLogo className="w-5 h-5 text-[var(--color-accent)]" />
        </div>
        <span className="font-serif text-xl font-semibold tracking-tight text-[var(--color-paper-on-desk)]">Aegis</span>
      </Link>

      <div className="w-full max-w-sm">{children}</div>
    </div>
  );
}
