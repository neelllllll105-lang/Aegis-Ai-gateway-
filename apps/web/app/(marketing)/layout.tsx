import Link from "next/link";

/** Marketing shell: a shared header and footer around the public pages. */
export default function MarketingLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-h-screen flex-col">
      <header className="sticky top-0 z-20 border-b border-[var(--color-line)] bg-[color-mix(in_srgb,var(--color-base)_88%,transparent)] backdrop-blur">
        <nav
          className="mx-auto flex max-w-5xl items-center justify-between px-6 py-3.5"
          aria-label="Main"
        >
          <Link href="/" className="flex items-center gap-2">
            <Shield />
            <span className="text-sm font-medium tracking-tight text-[var(--color-ink)]">
              Aegis
            </span>
          </Link>

          <div className="flex items-center gap-1">
            <NavLink href="/pricing">Pricing</NavLink>
            <NavLink href="/docs">Docs</NavLink>
            <Link
              href="/login"
              className="rounded-[var(--radius)] px-3 py-1.5 text-sm text-[var(--color-ink-muted)] transition-colors hover:text-[var(--color-ink)]"
            >
              Sign in
            </Link>
            <Link
              href="/signup"
              className="ml-1 rounded-[var(--radius)] bg-[var(--color-accent)] px-3 py-1.5 text-sm font-medium text-[#04140c] transition-colors hover:bg-[#4ee9a0]"
            >
              Get started
            </Link>
          </div>
        </nav>
      </header>

      <main className="flex-1">{children}</main>

      <footer className="border-t border-[var(--color-line)]">
        <div className="mx-auto max-w-5xl px-6 py-10">
          <div className="flex flex-wrap items-start justify-between gap-8">
            <div>
              <div className="flex items-center gap-2">
                <Shield />
                <span className="text-sm font-medium text-[var(--color-ink)]">Aegis</span>
              </div>
              <p className="mt-2 max-w-xs text-xs leading-relaxed text-[var(--color-ink-faint)]">
                An AI cost optimization gateway. Route smarter, cache harder, and see
                exactly what you saved.
              </p>
            </div>

            <div className="flex gap-12 text-sm">
              <FooterColumn
                heading="Product"
                links={[
                  { href: "/pricing", label: "Pricing" },
                  { href: "/docs", label: "Documentation" },
                  { href: "/status", label: "Status" },
                ]}
              />
              <FooterColumn
                heading="Account"
                links={[
                  { href: "/login", label: "Sign in" },
                  { href: "/signup", label: "Create account" },
                ]}
              />
            </div>
          </div>

          <div className="mt-10 border-t border-[var(--color-line)] pt-5 text-xs text-[var(--color-ink-faint)]">
            © {new Date().getFullYear()} Aegis. All rights reserved.
          </div>
        </div>
      </footer>
    </div>
  );
}

function NavLink({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <Link
      href={href}
      className="rounded-[var(--radius)] px-3 py-1.5 text-sm text-[var(--color-ink-muted)] transition-colors hover:text-[var(--color-ink)]"
    >
      {children}
    </Link>
  );
}

function FooterColumn({
  heading,
  links,
}: {
  heading: string;
  links: { href: string; label: string }[];
}) {
  return (
    <div>
      <div className="text-xs uppercase tracking-wide text-[var(--color-ink-subtle)]">
        {heading}
      </div>
      <ul className="mt-3 space-y-2">
        {links.map((link) => (
          <li key={link.href}>
            <Link
              href={link.href}
              className="text-[var(--color-ink-muted)] transition-colors hover:text-[var(--color-ink)]"
            >
              {link.label}
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** The mark. A shield, drawn rather than imported — one icon does not justify a library. */
function Shield() {
  return (
    <svg
      width="18"
      height="18"
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
  );
}
