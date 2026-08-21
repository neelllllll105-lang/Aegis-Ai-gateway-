"use client";

import { useState, type ReactNode } from "react";

// ---------------------------------------------------------------------------
// Aegis Sleek Minimalist Logo
// ---------------------------------------------------------------------------

export function AegisLogo({ className = "w-6 h-6" }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      className={className}
      aria-hidden="true"
    >
      <path
        d="M12 2L3.5 5.5V11C3.5 16.5 7.1 21.3 12 22.8C16.9 21.3 20.5 16.5 20.5 11V5.5L12 2Z"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M12 6.8L6.5 9.2V12.8C6.5 16.2 8.8 19.3 12 20.2C15.2 19.3 17.5 16.2 17.5 12.8V9.2L12 6.8Z"
        fill="currentColor"
        fillOpacity="0.25"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="12" cy="13" r="2" fill="currentColor" />
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
      ? "bg-[#EFE9E3] border border-[#D9CFC7]"
      : variant === "subtle"
        ? "bg-[#F9F8F6] border border-[#D9CFC7]"
        : "bg-white border border-[#D9CFC7]";

  return (
    <div
      className={`rounded-2xl ${variantClass} shadow-xs transition-all hover:border-[#C9B59C] ${className}`}
    >
      {children}
    </div>
  );
}

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
          <div className="font-mono text-[11px] font-black uppercase tracking-[0.2em] text-[#70685E] mb-1">
            {eyebrow}
          </div>
        )}
        <h2 className="text-2xl sm:text-3xl font-black tracking-tight text-black">
          {title}
        </h2>
        {description && (
          <p className="mt-1 text-sm text-[#403B35] max-w-2xl font-medium leading-relaxed">
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
    <div className="rounded-2xl border border-[#D9CFC7] bg-white p-5 shadow-xs relative overflow-hidden">
      <div className="flex items-center justify-between">
        <div className="text-[11px] font-black uppercase tracking-wider text-[#70685E]">
          {label}
        </div>
        {trend && (
          <span className="inline-flex items-center gap-1 text-[11px] font-black text-black bg-[#EFE9E3] px-2 py-0.5 rounded-full border border-[#D9CFC7]">
            {trend}
          </span>
        )}
      </div>
      <div
        className={`tabular mt-2 text-2xl sm:text-3xl font-black tracking-tight ${
          accent ? "text-[#0A0A0A]" : "text-black"
        }`}
      >
        {value}
      </div>
      {sublabel && (
        <div className="mt-1 text-xs text-[#70685E] font-medium">{sublabel}</div>
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
  neutral: "bg-[#EFE9E3] text-black border-[#D9CFC7] font-bold",
  accent: "bg-[#C9B59C] text-black border-[#BFAF98] font-black",
  warm: "bg-[#EFE9E3] text-black border-[#D9CFC7] font-bold",
  warn: "bg-[#FEF3C7] text-[#B45309] border-[#FDE68A] font-bold",
  danger: "bg-[#FEE2E2] text-[#DC2626] border-[#FECACA] font-bold",
  success: "bg-[#EFE9E3] text-black border-[#D9CFC7] font-bold",
  info: "bg-[#EFE9E3] text-black border-[#D9CFC7] font-bold",
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
    <div className="px-6 py-14 text-center rounded-2xl border border-dashed border-[#D9CFC7] bg-[#F9F8F6]">
      <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-full bg-[#EFE9E3] text-[#70685E] mb-3 border border-[#D9CFC7]">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
          <path d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
      </div>
      <p className="text-sm font-black text-black">{title}</p>
      <p className="mx-auto mt-1 max-w-md text-xs text-[#403B35] font-medium">
        {description}
      </p>
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

export function ErrorState({ message }: { message: string }) {
  return (
    <div className="border border-[#FECACA] bg-[#FEF2F2] rounded-xl p-4 flex items-start gap-3">
      <svg className="w-4 h-4 text-[#DC2626] shrink-0 mt-0.5" viewBox="0 0 20 20" fill="currentColor">
        <path fillRule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clipRule="evenodd" />
      </svg>
      <p className="text-xs font-bold text-[#DC2626]">{message}</p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

export function TableShell({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-x-auto rounded-2xl border border-[#D9CFC7] bg-white shadow-xs">
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
      className={`border-b border-[#D9CFC7] px-4 py-3.5 text-xs font-black uppercase tracking-wider text-black bg-[#EFE9E3] ${
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
      className={`border-b border-[#EFE9E3] px-4 py-3.5 text-xs ${align === "right" ? "text-right" : "text-left"} ${
        mono ? "tabular font-mono font-bold" : ""
      } ${muted ? "text-[#70685E]" : "text-black font-medium"}`}
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
    "bg-black text-white hover:bg-[#262626] shadow-xs font-black active:scale-[0.98]",
  accent:
    "bg-[#C9B59C] text-[#0A0A0A] hover:bg-[#BFAF98] shadow-xs font-bold active:scale-[0.98]",
  secondary:
    "bg-white text-black border border-[#D9CFC7] hover:bg-[#EFE9E3] shadow-xs font-bold active:scale-[0.98]",
  ghost:
    "text-[#403B35] hover:text-black hover:bg-[#EFE9E3] font-bold",
  danger:
    "bg-transparent text-[#DC2626] border border-[#FECACA] hover:bg-[#FEF2F2] font-bold",
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
        className="block text-xs font-black text-black"
      >
        {label}
        {required && <span className="ml-1 text-[#DC2626]">*</span>}
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
        className="mt-1.5 w-full rounded-xl border border-[#D9CFC7] bg-white px-3.5 py-2.5 text-xs text-black placeholder:text-[#9E9487] focus:border-[#C9B59C] focus:ring-1 focus:ring-[#C9B59C] focus:outline-none transition-colors"
      />
      {hint && <p className="mt-1 text-[11px] text-[#70685E] font-medium">{hint}</p>}
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
          <span key={idx} className="block text-[#C9B59C] font-medium">
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
            <span className="text-[#F9F8F6]">{codePart}</span>
            <span className="text-[#C9B59C] font-bold">{commentPart}</span>
          </span>
        );
      }

      return (
        <span key={idx} className="block text-[#F9F8F6]">
          {line}
        </span>
      );
    });
  }

  return (
    <div className="relative group rounded-2xl border border-[#38332D] bg-[#141210] text-[#F9F8F6] overflow-hidden shadow-sm">
      {/* Terminal Window Header */}
      <div className="flex items-center justify-between px-4 py-2.5 bg-[#0D0B0A] border-b border-[#2A2420] text-xs">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5">
            <span
              className="w-3 h-3 rounded-full bg-[#FF5F56] border border-[#E0443E]"
              title="Close"
            />
            <span
              className="w-3 h-3 rounded-full bg-[#FFBD2E] border border-[#DEA123]"
              title="Minimize"
            />
            <span
              className="w-3 h-3 rounded-full bg-[#27C93F] border border-[#1AAB29]"
              title="Expand"
            />
          </div>
          {caption && <span className="ml-1 font-bold text-[#F9F8F6]">{caption}</span>}
        </div>
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-mono uppercase tracking-wider text-[#A8A29E] font-bold">
            {language}
          </span>
          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center gap-1 rounded bg-[#2A2420] px-2 py-1 text-[10px] font-bold text-[#F9F8F6] hover:bg-[#403B35] transition-colors"
            aria-label="Copy code"
          >
            {copied ? (
              <span className="text-[#C9B59C]">Copied</span>
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
