import Link from "next/link";

/** Auth shell: a single centred card with nothing else competing for attention. */
export default function AuthLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center px-6 py-12">
      <Link href="/" className="mb-8 flex items-center gap-2">
        <svg
          width="20"
          height="20"
          viewBox="0 0 20 20"
          fill="none"
          aria-hidden="true"
          className="text-[var(--color-accent)]"
        >
          <path
            d="M10 1.5 3 4.2v5.3c0 4.2 2.9 7.5 7 9 4.1-1.5 7-4.8 7-9V4.2L10 1.5Z"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
          <path
            d="m6.8 9.8 2.3 2.3 4.1-4.6"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        <span className="font-medium tracking-tight text-[var(--color-ink)]">Aegis</span>
      </Link>

      <div className="w-full max-w-sm">{children}</div>
    </div>
  );
}
