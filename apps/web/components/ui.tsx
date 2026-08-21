/**
 * Shared primitives.
 *
 * Small and deliberately unclever. Every component here exists because the same markup
 * appeared three or more times; nothing is here speculatively. Variants are closed unions
 * rather than open strings, so an invalid state is a type error and not a silently
 * unstyled element.
 */

import type { ReactNode } from "react";

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/** A bordered panel. The only container style in the product. */
export function Card({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return <div className={`card ${className}`}>{children}</div>;
}

/** A section heading with optional supporting text and a trailing action. */
export function SectionHeader({
  title,
  description,
  action,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-6 mb-6">
      <div>
        <h2 className="text-lg font-medium text-[var(--color-ink)]">{title}</h2>
        {description && (
          <p className="mt-1 text-sm text-[var(--color-ink-subtle)] max-w-2xl">
            {description}
          </p>
        )}
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Data display
// ---------------------------------------------------------------------------

/**
 * A headline metric.
 *
 * The value is monospace and tabular so a column of tiles stays aligned and a
 * live-updating figure does not jitter as digit widths change.
 */
export function Stat({
  label,
  value,
  sublabel,
  accent = false,
}: {
  label: string;
  value: string;
  sublabel?: string;
  accent?: boolean;
}) {
  return (
    <Card className="p-5">
      <div className="text-xs uppercase tracking-wide text-[var(--color-ink-subtle)]">
        {label}
      </div>
      <div
        className={`tabular mt-2 text-2xl ${
          accent ? "text-[var(--color-accent)]" : "text-[var(--color-ink)]"
        }`}
      >
        {value}
      </div>
      {sublabel && (
        <div className="mt-1 text-xs text-[var(--color-ink-faint)]">{sublabel}</div>
      )}
    </Card>
  );
}

type BadgeTone = "neutral" | "accent" | "warn" | "danger" | "info";

const BADGE_TONES: Record<BadgeTone, string> = {
  neutral:
    "bg-[var(--color-raised)] text-[var(--color-ink-muted)] border-[var(--color-line)]",
  accent:
    "bg-[var(--color-accent-wash)] text-[var(--color-accent)] border-[var(--color-accent-dim)]",
  warn: "bg-[#2a2113] text-[var(--color-warn)] border-[#4a3a1c]",
  danger: "bg-[#2a1616] text-[var(--color-danger)] border-[#4a2424]",
  info: "bg-[#131f2f] text-[var(--color-info)] border-[#23364f]",
};

/** A small status pill. */
export function Badge({
  children,
  tone = "neutral",
}: {
  children: ReactNode;
  tone?: BadgeTone;
}) {
  return (
    <span
      className={`inline-flex items-center rounded border px-1.5 py-0.5 text-xs font-medium ${BADGE_TONES[tone]}`}
    >
      {children}
    </span>
  );
}

/**
 * An empty state.
 *
 * Always says what to do next. A panel reading only "No data" tells the user nothing
 * about whether the product is broken or simply new.
 */
export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="px-6 py-14 text-center">
      <p className="text-sm font-medium text-[var(--color-ink-muted)]">{title}</p>
      <p className="mx-auto mt-1 max-w-md text-sm text-[var(--color-ink-subtle)]">
        {description}
      </p>
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

/** An error panel. */
export function ErrorState({ message }: { message: string }) {
  return (
    <Card className="border-[#4a2424] bg-[#1a1112] p-5">
      <p className="text-sm text-[var(--color-danger)]">{message}</p>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

/**
 * A table wrapper that scrolls horizontally on narrow screens.
 *
 * Without the overflow container, a wide request log pushes the whole page sideways on a
 * phone and every other page element goes with it.
 */
export function TableShell({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full min-w-[640px] text-sm">{children}</table>
    </div>
  );
}

/** A table header cell. */
export function Th({
  children,
  align = "left",
}: {
  children: ReactNode;
  align?: "left" | "right";
}) {
  return (
    <th
      className={`hairline px-4 py-2.5 text-xs font-medium uppercase tracking-wide text-[var(--color-ink-subtle)] ${
        align === "right" ? "text-right" : "text-left"
      }`}
    >
      {children}
    </th>
  );
}

/** A table body cell. */
export function Td({
  children,
  align = "left",
  mono = false,
  muted = false,
}: {
  children: ReactNode;
  align?: "left" | "right";
  mono?: boolean;
  muted?: boolean;
}) {
  return (
    <td
      className={`hairline px-4 py-2.5 ${align === "right" ? "text-right" : "text-left"} ${
        mono ? "tabular" : ""
      } ${muted ? "text-[var(--color-ink-subtle)]" : "text-[var(--color-ink-muted)]"}`}
    >
      {children}
    </td>
  );
}

// ---------------------------------------------------------------------------
// Forms
// ---------------------------------------------------------------------------

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary:
    "bg-[var(--color-accent)] text-[#04140c] hover:bg-[#4ee9a0] font-medium",
  secondary:
    "bg-[var(--color-raised)] text-[var(--color-ink)] border border-[var(--color-line-strong)] hover:bg-[var(--color-overlay)]",
  ghost:
    "text-[var(--color-ink-muted)] hover:text-[var(--color-ink)] hover:bg-[var(--color-raised)]",
  danger:
    "bg-transparent text-[var(--color-danger)] border border-[#4a2424] hover:bg-[#2a1616]",
};

/** A button. */
export function Button({
  children,
  variant = "primary",
  type = "button",
  disabled = false,
  onClick,
  className = "",
}: {
  children: ReactNode;
  variant?: ButtonVariant;
  type?: "button" | "submit";
  disabled?: boolean;
  onClick?: () => void;
  className?: string;
}) {
  return (
    <button
      type={type}
      disabled={disabled}
      onClick={onClick}
      className={`inline-flex items-center justify-center gap-2 rounded-[var(--radius)] px-3 py-1.5 text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${BUTTON_VARIANTS[variant]} ${className}`}
    >
      {children}
    </button>
  );
}

/**
 * A labelled text field.
 *
 * The label is a real `<label>` bound by `htmlFor`, not a placeholder. Placeholder-only
 * fields lose their label the moment the user types, which is exactly when they are still
 * needed.
 */
export function Field({
  label,
  id,
  type = "text",
  value,
  onChange,
  placeholder,
  hint,
  required = false,
  autoComplete,
}: {
  label: string;
  id: string;
  type?: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  hint?: string;
  required?: boolean;
  autoComplete?: string;
}) {
  return (
    <div>
      <label
        htmlFor={id}
        className="block text-sm font-medium text-[var(--color-ink-muted)]"
      >
        {label}
        {required && <span className="ml-1 text-[var(--color-ink-faint)]">*</span>}
      </label>
      <input
        id={id}
        name={id}
        type={type}
        value={value}
        required={required}
        autoComplete={autoComplete}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
        className="mt-1.5 w-full rounded-[var(--radius)] border border-[var(--color-line-strong)] bg-[var(--color-base)] px-3 py-2 text-sm text-[var(--color-ink)] placeholder:text-[var(--color-ink-faint)] focus:border-[var(--color-accent-dim)] focus:outline-none focus-visible:outline-none"
      />
      {hint && <p className="mt-1.5 text-xs text-[var(--color-ink-subtle)]">{hint}</p>}
    </div>
  );
}

/** A block of code with a caption. */
export function CodeBlock({
  code,
  caption,
}: {
  code: string;
  caption?: string;
}) {
  return (
    <div>
      {caption && (
        <div className="mb-1.5 text-xs text-[var(--color-ink-subtle)]">{caption}</div>
      )}
      <pre className="overflow-x-auto rounded-[var(--radius)] border border-[var(--color-line)] bg-[var(--color-base)] p-4 text-xs leading-relaxed text-[var(--color-ink-muted)]">
        <code>{code}</code>
      </pre>
    </div>
  );
}
