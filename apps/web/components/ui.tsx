"use client";

import { useRouter } from "next/navigation";
import { useState, type ReactNode } from "react";

// ---------------------------------------------------------------------------
// Aegis Bespoke Sentinel Prism Logo
// Handcrafted editorial vector emblem: dual-faceted architectural shield,
// interlocking routing prism keystone, and precision gold/terracotta core.
// ---------------------------------------------------------------------------

export function AegisLogo({
  className = "w-6 h-6",
}: {
  className?: string;
}) {
  return (
    <svg
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-hidden="true"
    >
      <defs>
        {/* Editorial Linear Gradients tailored to the Aegis Deskwork Palette */}
        <linearGradient
          id="aegis-facet-left"
          x1="6"
          y1="5"
          x2="16"
          y2="28"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0%" stopColor="var(--color-accent, #A8341E)" />
          <stop offset="100%" stopColor="var(--color-accent-dark, #822712)" />
        </linearGradient>
        <linearGradient
          id="aegis-facet-right"
          x1="26"
          y1="5"
          x2="16"
          y2="28"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0%" stopColor="var(--color-ink, #211C14)" />
          <stop offset="100%" stopColor="var(--color-muted, #55492F)" />
        </linearGradient>
        <linearGradient
          id="aegis-core-glow"
          x1="16"
          y1="8"
          x2="16"
          y2="24"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0%" stopColor="var(--color-ochre, #A07A1E)" />
          <stop offset="100%" stopColor="var(--color-accent, #A8341E)" />
        </linearGradient>
      </defs>

      {/* Outer Faceted Shield Frame */}
      <path
        d="M16 2.5L5 6.5V14C5 21.2 9.7 27.8 16 30C22.3 27.8 27 21.2 27 14V6.5L16 2.5Z"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />

      {/* Left Facet Wing - Architectural Terracotta Plane */}
      <path
        d="M16 4.8L7.2 8.2V13.8C7.2 19.6 10.9 24.8 16 26.8V16L16 4.8Z"
        fill="url(#aegis-facet-left)"
        fillOpacity="0.88"
      />

      {/* Right Facet Wing - Architectural Ink/Ochre Plane */}
      <path
        d="M16 4.8L24.8 8.2V13.8C24.8 19.6 21.1 24.8 16 26.8V16L16 4.8Z"
        fill="url(#aegis-facet-right)"
        fillOpacity="0.82"
      />

      {/* The Central Geometric Nexus Prism (The Routing Diamond) */}
      <path
        d="M16 8.5L20.8 14.8L16 23.2L11.2 14.8L16 8.5Z"
        fill="var(--color-surface, #FBF8F1)"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />

      {/* The Core Intelligent Pulse Spark (The Sentinel Nexus) */}
      <path
        d="M16 11.2L18.8 15L16 19.8L13.2 15L16 11.2Z"
        fill="url(#aegis-core-glow)"
      />

      {/* Center Keystone Focus */}
      <circle cx="16" cy="15" r="1.25" fill="var(--color-surface, #FBF8F1)" />
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Layout & Panels
// ---------------------------------------------------------------------------

export function Card({
  children,
  className = "",
  variant = "default",
}: {
  children: ReactNode;
  className?: string;
  variant?: "default" | "warm" | "subtle";
}) {
  const variantClass =
    variant === "warm"
      ? "bg-[var(--color-surface2)] border-[var(--color-ink)] text-[var(--color-ink)]"
      : variant === "subtle"
        ? "bg-[var(--color-desk-raised)] border-[var(--color-desk-line)] text-[var(--color-paper-on-desk)]"
        : "bg-[var(--color-surface)] border-[var(--color-ink)] text-[var(--color-ink)]";

  return (
    <div
      className={`rounded-2xl border-[1.5px] ${variantClass} shadow-[3px_3px_0_var(--shadow-color)] transition-transform duration-150 hover:-translate-x-px hover:-translate-y-px ${className}`}
    >
      {children}
    </div>
  );
}

/**
 * Always used bare, directly on the dashboard's dark desk main area — never inside a
 * paper card. Its text colors are therefore the `*-on-desk` trio, not `--color-ink`.
 * If this ever gets used inside a `Card`, it will need a `variant` prop instead.
 */
export function SectionHeader({
  title,
  description,
  action,
  eyebrow,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
  eyebrow?: string;
}) {
  return (
    <div className="flex flex-col sm:flex-row sm:items-start justify-between gap-4 mb-6">
      <div>
        {eyebrow && (
          <div className="font-mono text-[11px] uppercase tracking-[0.2em] font-bold text-[var(--color-muted-on-desk)] mb-1">
            {eyebrow}
          </div>
        )}
        <h2 className="font-serif text-2xl sm:text-3xl font-semibold tracking-tight text-[var(--color-paper-on-desk)]">
          {title}
        </h2>
        {description && (
          <p className="mt-1 text-sm text-[var(--color-muted-on-desk)] max-w-2xl font-medium leading-relaxed font-sans">
            {description}
          </p>
        )}
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Data Display
// ---------------------------------------------------------------------------

export function Stat({
  label,
  value,
  sublabel,
  accent = false,
  trend,
}: {
  label: string;
  value: string;
  sublabel?: string;
  accent?: boolean;
  trend?: string;
}) {
  return (
    <div className="rounded-2xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface)] p-5 shadow-[3px_3px_0_var(--shadow-color)] relative overflow-hidden">
      <div className="flex items-center justify-between">
        <div className="eyebrow-accent">{label}</div>
        {trend && (
          <span className="inline-flex items-center gap-1 text-[11px] font-mono font-bold text-[var(--color-ink)] bg-[var(--color-surface2)] px-2 py-0.5 rounded-full border border-[var(--color-ink)]">
            {trend}
          </span>
        )}
      </div>
      <div
        className={`tabular mt-2 text-2xl sm:text-3xl font-semibold tracking-tight font-serif ${
          accent ? "text-[var(--color-accent)]" : "text-[var(--color-ink)]"
        }`}
      >
        {value}
      </div>
      {sublabel && (
        <div className="mt-1 text-xs text-[var(--color-muted-light)] font-medium">{sublabel}</div>
      )}
    </div>
  );
}

export type BadgeTone =
  | "neutral"
  | "accent"
  | "warm"
  | "warn"
  | "danger"
  | "success"
  | "info";

const BADGE_TONES: Record<BadgeTone, string> = {
  neutral: "bg-[var(--color-surface2)] text-[var(--color-ink)] border-[var(--color-ink)] font-semibold",
  accent: "bg-[var(--color-accent)] text-[var(--color-surface)] border-[var(--color-accent-dark)] font-bold",
  warm: "bg-[var(--color-surface2)] text-[var(--color-ink)] border-[var(--color-ink)] font-semibold",
  warn: "bg-[var(--color-amber-bg)] text-[var(--color-amber)] border-[var(--color-amber)] font-bold",
  danger: "bg-[var(--color-accent-bg)] text-[var(--color-accent)] border-[var(--color-accent)] font-bold",
  success: "bg-[var(--color-positive-bg)] text-[var(--color-positive)] border-[var(--color-positive)] font-bold",
  info: "bg-[var(--color-ochre-bg)] text-[var(--color-ochre)] border-[var(--color-ochre)] font-bold",
};

export function Badge({
  children,
  tone = "neutral",
  size = "md",
}: {
  children: ReactNode;
  tone?: BadgeTone;
  size?: "sm" | "md";
}) {
  const sizeClasses = size === "sm" ? "px-2 py-0.5 text-[10px]" : "px-3 py-1 text-xs";
  return (
    <span
      className={`inline-flex items-center gap-1.5 font-bold rounded-full border ${BADGE_TONES[tone]} ${sizeClasses}`}
    >
      {children}
    </span>
  );
}

/**
 * Stamp — a rotated, bordered one-word chip for a routing reason, a cache
 * outcome, a plan tier. The border always matches the text color (see
 * `.stamp` in globals.css), so a single `tone` sets the whole chip.
 */
export type StampTone = "agent" | "pending" | "verdict" | "muted" | "other";

const STAMP_TONES: Record<StampTone, string> = {
  agent: "text-[var(--color-accent)]",
  pending: "text-[var(--color-amber)]",
  verdict: "text-[var(--color-positive)]",
  muted: "text-[var(--color-muted-light)]",
  other: "text-[var(--color-ochre)]",
};

export function Stamp({
  children,
  tone = "muted",
}: {
  children: ReactNode;
  tone?: StampTone;
}) {
  return <span className={`stamp ${STAMP_TONES[tone]}`}>{children}</span>;
}

/**
 * AttributionChip — who is responsible for this value: the caller (YOU),
 * the routing engine (AEGIS), or an upstream party (a provider name). This
 * is the on-brand home for the requested-vs-served model pair.
 */
export type AttributionActor = "you" | "agent" | "other";

const ATTRIBUTION_STYLES: Record<AttributionActor, string> = {
  you: "bg-[var(--color-ink)] text-[var(--color-surface)]",
  agent: "bg-[var(--color-accent)] text-[var(--color-surface)]",
  other: "bg-[var(--color-ochre)] text-[var(--color-ink)]",
};

export function AttributionChip({
  actor,
  label,
}: {
  actor: AttributionActor;
  label: string;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[10px] font-mono font-bold uppercase tracking-wider ${ATTRIBUTION_STYLES[actor]}`}
    >
      {label}
    </span>
  );
}

/**
 * DecisionCard — narrates one thing the routing engine did: who/what acted,
 * what it did, when, the exact value, and why. The left rail color is the
 * outcome at a glance; everything else is read left-to-right like a log
 * line that happens to be legible.
 */
export type DecisionOutcome = "agent" | "pending" | "verdict" | "muted" | "other";

const DECISION_RAIL: Record<DecisionOutcome, string> = {
  agent: "rail-agent",
  pending: "rail-pending",
  verdict: "rail-verdict",
  muted: "rail-muted",
  other: "rail-other",
};

export function DecisionCard({
  actor,
  action,
  timestamp,
  value,
  reason,
  outcome = "muted",
}: {
  actor: string;
  action: string;
  timestamp: string;
  value?: string;
  reason?: string;
  outcome?: DecisionOutcome;
}) {
  return (
    <div
      className={`${DECISION_RAIL[outcome]} bg-[var(--color-surface)] border border-[var(--color-line)] rounded-lg px-4 py-3`}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-xs">
          <span className="font-bold text-[var(--color-ink)]">{actor}</span>
          <span className="text-[var(--color-muted-light)]">{action}</span>
        </div>
        <span className="tabular font-mono text-[10px] text-[var(--color-muted-light)] shrink-0">
          {timestamp}
        </span>
      </div>
      {value && (
        <div className="tabular mt-1 font-mono text-sm font-bold text-[var(--color-ink)]">{value}</div>
      )}
      {reason && (
        <div className="mt-1 text-xs text-[var(--color-muted)] leading-relaxed">{reason}</div>
      )}
    </div>
  );
}

/**
 * DisclosureMeter — a spend/usage bar where the solid fill is committed
 * spend and a diagonal hatch is reserved-but-not-final (an atomic budget
 * hold, a projection). Reading the hatch as "not settled yet" is the whole
 * point — don't replace it with a second solid color.
 */
export function DisclosureMeter({
  label,
  committedPct,
  reservedPct = 0,
  tone = "verdict",
}: {
  label?: string;
  committedPct: number;
  reservedPct?: number;
  tone?: "verdict" | "pending" | "agent";
}) {
  const fillColor =
    tone === "pending"
      ? "var(--color-amber)"
      : tone === "agent"
        ? "var(--color-accent)"
        : "var(--color-positive)";
  const committed = Math.max(0, Math.min(100, committedPct));
  const reserved = Math.max(0, Math.min(100 - committed, reservedPct));

  return (
    <div>
      {label && (
        <div className="mb-1.5 flex items-center justify-between text-[11px] font-mono uppercase tracking-wider text-[var(--color-muted-light)]">
          <span>{label}</span>
          <span className="tabular">{Math.round(committed + reserved)}%</span>
        </div>
      )}
      <div className="flex h-2.5 w-full overflow-hidden rounded-full border border-[var(--color-ink)] bg-[var(--color-surface2)]">
        <div style={{ width: `${committed}%`, backgroundColor: fillColor }} />
        <div
          style={{
            width: `${reserved}%`,
            backgroundImage: `repeating-linear-gradient(45deg, ${fillColor} 0, ${fillColor} 2px, transparent 2px, transparent 6px)`,
          }}
        />
      </div>
    </div>
  );
}

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
    <div className="border-pending rounded-2xl border-[1.5px] bg-[var(--color-surface)] px-6 py-14 text-center">
      <div className="mx-auto mb-3 flex h-10 w-10 items-center justify-center rounded-full border border-[var(--color-ink)] bg-[var(--color-surface2)] text-[var(--color-muted-light)]">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
          <path d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
      </div>
      <p className="font-serif text-sm font-semibold text-[var(--color-ink)]">{title}</p>
      <p className="mx-auto mt-1 max-w-md text-xs text-[var(--color-muted)] font-medium">
        {description}
      </p>
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

/**
 * Shown in place of a page's real content when the org's plan doesn't include the
 * feature — defense in depth for a direct URL visit, since the sidebar already hides the
 * nav entry that would normally lead here. `feature`/`requiredPlan` should read as plain
 * English ("Budgets" / "Team"), not the machine-readable `PlanFeature` key.
 */
export function UpgradeRequired({
  feature,
  requiredPlan,
}: {
  feature: string;
  requiredPlan: string;
}) {
  const router = useRouter();
  return (
    <div className="border-pending rounded-2xl border-[1.5px] bg-[var(--color-surface)] px-6 py-14 text-center">
      <div className="mx-auto mb-3 flex h-10 w-10 items-center justify-center rounded-full border border-[var(--color-ink)] bg-[var(--color-surface2)] text-[var(--color-muted-light)]">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v2h8z" />
        </svg>
      </div>
      <p className="font-serif text-sm font-semibold text-[var(--color-ink)]">
        {feature} needs the {requiredPlan} plan
      </p>
      <p className="mx-auto mt-1 max-w-md text-xs text-[var(--color-muted)] font-medium">
        Nothing about your current usage or data changes until you upgrade — this is the
        only thing that does.
      </p>
      <div className="mt-5">
        <Button variant="accent" onClick={() => router.push("/billing")}>
          View plans
        </Button>
      </div>
    </div>
  );
}

export function ErrorState({ message }: { message: string }) {
  return (
    <div className="rail-agent flex items-start gap-3 rounded-xl border border-[var(--color-line)] bg-[var(--color-accent-bg)] p-4">
      <svg className="w-4 h-4 text-[var(--color-accent)] shrink-0 mt-0.5" viewBox="0 0 20 20" fill="currentColor">
        <path fillRule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clipRule="evenodd" />
      </svg>
      <p className="text-xs font-bold text-[var(--color-accent)]">{message}</p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

export function TableShell({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-x-auto rounded-2xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)]">
      <table className="w-full min-w-[640px] text-sm">{children}</table>
    </div>
  );
}

export function Th({
  children,
  align = "left",
}: {
  children: ReactNode;
  align?: "left" | "right";
}) {
  return (
    <th
      className={`border-b-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface2)] px-4 py-3.5 font-mono text-xs font-bold uppercase tracking-wider text-[var(--color-ink)] ${
        align === "right" ? "text-right" : "text-left"
      }`}
    >
      {children}
    </th>
  );
}

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
      className={`border-b border-[var(--color-line)] px-4 py-3.5 text-xs ${align === "right" ? "text-right" : "text-left"} ${
        mono ? "tabular font-mono font-bold" : ""
      } ${muted ? "text-[var(--color-muted-light)]" : "text-[var(--color-ink)] font-medium"}`}
    >
      {children}
    </td>
  );
}

// ---------------------------------------------------------------------------
// Forms & Buttons
// ---------------------------------------------------------------------------

type ButtonVariant = "primary" | "secondary" | "accent" | "ghost" | "danger";

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary:
    "bg-[var(--color-ink)] text-[var(--color-surface)] border-[1.5px] border-[var(--color-ink)] shadow-[3px_3px_0_var(--shadow-color)] hover:-translate-x-px hover:-translate-y-px hover:shadow-[4px_4px_0_var(--shadow-color)] active:translate-x-px active:translate-y-px active:shadow-[1px_1px_0_var(--shadow-color)] font-bold",
  accent: "btn-accent !px-5 !py-2.5 !rounded-xl",
  secondary:
    "bg-[var(--color-surface)] text-[var(--color-ink)] border-[1.5px] border-[var(--color-ink)] hover:bg-[var(--color-surface2)] font-bold",
  ghost:
    "text-[var(--color-muted)] hover:text-[var(--color-ink)] hover:bg-[var(--color-surface2)] font-bold",
  danger:
    "bg-transparent text-[var(--color-accent)] border-[1.5px] border-[var(--color-accent)] hover:bg-[var(--color-accent-bg)] font-bold",
};

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
      className={`inline-flex items-center justify-center gap-2 rounded-xl px-5 py-2.5 text-xs transition-all duration-150 disabled:cursor-not-allowed disabled:opacity-50 ${BUTTON_VARIANTS[variant]} ${className}`}
    >
      {children}
    </button>
  );
}

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
        className="block text-xs font-bold text-[var(--color-ink)]"
      >
        {label}
        {required && <span className="ml-1 text-[var(--color-accent)]">*</span>}
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
        className="mt-1.5 w-full rounded-lg border-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface)] px-3.5 py-2.5 text-xs text-[var(--color-ink)] placeholder:text-[var(--color-muted-light)] focus:border-[var(--color-accent)] focus:ring-1 focus:ring-[var(--color-accent)] focus:outline-none transition-colors"
      />
      {hint && <p className="mt-1 text-[11px] text-[var(--color-muted-light)] font-medium">{hint}</p>}
    </div>
  );
}

/**
 * Terminal CodeBlock with authentic window traffic light controls and crisp syntax colors.
 */
export function CodeBlock({
  code,
  caption,
  language = "bash",
}: {
  code: string;
  caption?: string;
  language?: string;
}) {
  const [copied, setCopied] = useState(false);

  function handleCopy() {
    navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  function formatHighlightedCode(rawCode: string) {
    const lines = rawCode.split("\n");
    return lines.map((line, idx) => {
      // Comments
      if (line.trim().startsWith("#")) {
        return (
          <span key={idx} className="block text-[#B8912E] font-medium">
            {line}
          </span>
        );
      }

      const commentIndex = line.indexOf("#");
      if (commentIndex !== -1) {
        const codePart = line.slice(0, commentIndex);
        const commentPart = line.slice(commentIndex);
        return (
          <span key={idx} className="block">
            <span className="text-[#F7F1E4]">{codePart}</span>
            <span className="text-[#B8912E] font-bold">{commentPart}</span>
          </span>
        );
      }

      return (
        <span key={idx} className="block text-[#F7F1E4]">
          {line}
        </span>
      );
    });
  }

  return (
    <div className="relative group rounded-2xl border-[1.5px] border-[#3C3324] bg-[#201B14] text-[#F7F1E4] overflow-hidden shadow-[4px_4px_0_var(--shadow-color)]">
      {/* Terminal Window Header */}
      <div className="flex items-center justify-between px-4 py-2.5 bg-[#160F09] border-b border-[#3C3324] text-xs">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5">
            <span
              className="w-3 h-3 rounded-full bg-[#A8341E] border border-[#822712]"
              title="Close"
            />
            <span
              className="w-3 h-3 rounded-full bg-[#B8912E] border border-[#93630F]"
              title="Minimize"
            />
            <span
              className="w-3 h-3 rounded-full bg-[#5B8A4E] border border-[#3F6B34]"
              title="Expand"
            />
          </div>
          {caption && <span className="ml-1 font-bold text-[#F7F1E4]">{caption}</span>}
        </div>
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-mono uppercase tracking-wider text-[#B8AC8E] font-bold">
            {language}
          </span>
          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center gap-1 rounded bg-[#3C3324] px-2 py-1 text-[10px] font-bold text-[#F7F1E4] hover:bg-[#4A3F2C] transition-colors"
            aria-label="Copy code"
          >
            {copied ? (
              <span className="text-[#B8912E]">Copied</span>
            ) : (
              <span>Copy</span>
            )}
          </button>
        </div>
      </div>
      <pre className="overflow-x-auto p-4 text-xs leading-relaxed font-mono">
        <code>{formatHighlightedCode(code)}</code>
      </pre>
    </div>
  );
}
