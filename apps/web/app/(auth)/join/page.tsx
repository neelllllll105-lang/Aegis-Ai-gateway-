"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState } from "react";
import { api, ApiError } from "@/lib/api";
import { Button, Card, ErrorState, Field } from "@/components/ui";

const MIN_PASSWORD_LENGTH = 12;

function JoinForm() {
  const router = useRouter();
  const searchParams = useSearchParams();

  const token = searchParams.get("token") ?? "";
  const emailParam = searchParams.get("email") ?? "";

  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const passwordTooShort = password.length > 0 && password.length < MIN_PASSWORD_LENGTH;
  const passwordsMismatch = confirmPassword.length > 0 && password !== confirmPassword;

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    setError(null);

    if (!token) {
      setError("Missing invitation token. Please check the link from your email.");
      return;
    }

    if (password.length < MIN_PASSWORD_LENGTH) {
      setError(`Password must be at least ${MIN_PASSWORD_LENGTH} characters.`);
      return;
    }

    if (password !== confirmPassword) {
      setError("Passwords do not match.");
      return;
    }

    setSubmitting(true);
    try {
      await api.acceptInvite(token, password, name || undefined);
      router.push("/dashboard");
    } catch (caught) {
      setError(
        caught instanceof ApiError
          ? caught.message
          : "Could not activate your account. Please verify your connection or request a new invitation.",
      );
      setSubmitting(false);
    }
  }

  return (
    <Card className="p-7">
      <div className="flex items-center gap-2 mb-2">
        <span className="flex h-2.5 w-2.5 rounded-full bg-[var(--color-accent)]" />
        <span className="text-[11px] font-mono font-bold tracking-widest uppercase text-[var(--color-accent)]">
          Team Invitation
        </span>
      </div>

      <h1 className="font-serif text-xl font-semibold tracking-tight text-[var(--color-ink)]">
        Accept Invitation
      </h1>
      <p className="mt-1 text-xs text-[var(--color-muted)] font-medium">
        {emailParam
          ? `You were invited as ${emailParam}. Set your password to activate your access.`
          : "Set your password to activate your account and access the dashboard."}
      </p>

      <form onSubmit={handleSubmit} className="mt-6 space-y-4">
        {emailParam && (
          <div>
            <label className="block text-xs font-bold text-[var(--color-muted)] mb-1">
              Email Address
            </label>
            <div className="w-full rounded-xl border border-[var(--color-desk-line)] bg-[var(--color-surface2)] px-3 py-2 text-xs font-mono font-bold text-[var(--color-ink)]">
              {emailParam}
            </div>
          </div>
        )}

        <Field
          label="Full Name (Optional)"
          id="name"
          value={name}
          onChange={setName}
          autoComplete="name"
          placeholder="Jane Doe"
        />

        <div>
          <Field
            label="Set Password"
            id="password"
            type="password"
            value={password}
            onChange={setPassword}
            autoComplete="new-password"
            placeholder="At least 12 characters"
            required
          />
          {passwordTooShort && (
            <p className="mt-1 text-[11px] text-[var(--color-amber)] font-medium">
              Must be at least {MIN_PASSWORD_LENGTH} characters ({password.length}/{MIN_PASSWORD_LENGTH})
            </p>
          )}
        </div>

        <div>
          <Field
            label="Confirm Password"
            id="confirm-password"
            type="password"
            value={confirmPassword}
            onChange={setConfirmPassword}
            autoComplete="new-password"
            placeholder="Repeat password"
            required
          />
          {passwordsMismatch && (
            <p className="mt-1 text-[11px] text-[var(--color-amber)] font-medium">
              Passwords do not match
            </p>
          )}
        </div>

        {error && <ErrorState message={error} />}

        <Button
          type="submit"
          disabled={submitting || passwordTooShort || passwordsMismatch || !token}
          className="w-full mt-2"
        >
          {submitting ? "Activating Account…" : "Activate Account & Join Team"}
        </Button>
      </form>

      <p className="mt-6 text-center text-xs text-[var(--color-muted)] font-medium">
        Already have a configured account?{" "}
        <Link
          href="/login"
          className="text-[var(--color-accent)] font-bold hover:underline"
        >
          Sign in
        </Link>
      </p>
    </Card>
  );
}

export default function JoinPage() {
  return (
    <Suspense
      fallback={
        <Card className="p-7">
          <div className="flex items-center justify-center p-8 text-xs font-bold text-[var(--color-muted)]">
            Loading invitation…
          </div>
        </Card>
      }
    >
      <JoinForm />
    </Suspense>
  );
}
