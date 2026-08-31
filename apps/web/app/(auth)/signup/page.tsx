"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { api, ApiError } from "@/lib/api";
import { Button, Card, ErrorState, Field } from "@/components/ui";

const MIN_PASSWORD_LENGTH = 12;

export default function SignupPage() {
  const router = useRouter();
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const passwordTooShort = password.length > 0 && password.length < MIN_PASSWORD_LENGTH;

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    setError(null);

    if (password.length < MIN_PASSWORD_LENGTH) {
      setError(`Password must be at least ${MIN_PASSWORD_LENGTH} characters.`);
      return;
    }

    setSubmitting(true);
    try {
      await api.signup(email, password, name || undefined);
      router.push("/dashboard");
    } catch (caught) {
      setError(
        caught instanceof ApiError
          ? caught.message
          : "Could not reach the server. Check your connection and try again.",
      );
      setSubmitting(false);
    }
  }

  return (
    <Card className="p-7">
      <h1 className="font-serif text-xl font-semibold tracking-tight text-[var(--color-ink)]">
        Create free account
      </h1>
      <p className="mt-1 text-xs text-[var(--color-muted)] font-medium">
        10,000 free requests per month. No credit card required.
      </p>

      <form onSubmit={handleSubmit} className="mt-6 space-y-4">
        <Field
          label="Full Name"
          id="name"
          value={name}
          onChange={setName}
          autoComplete="name"
          placeholder="Jane Doe (Optional)"
        />
        <Field
          label="Work Email"
          id="email"
          type="email"
          value={email}
          onChange={setEmail}
          autoComplete="email"
          placeholder="name@company.com"
          required
        />
        <Field
          label="Password"
          id="password"
          type="password"
          value={password}
          onChange={setPassword}
          autoComplete="new-password"
          required
          hint={
            passwordTooShort
              ? `${MIN_PASSWORD_LENGTH - password.length} more characters needed`
              : `Minimum ${MIN_PASSWORD_LENGTH} characters.`
          }
        />

        {error && <ErrorState message={error} />}

        <Button type="submit" disabled={submitting} className="w-full mt-2">
          {submitting ? "Creating account…" : "Get Started Free"}
        </Button>
      </form>

      <p className="mt-6 text-center text-xs text-[var(--color-muted)] font-medium">
        Already have an account?{" "}
        <Link href="/login" className="font-bold text-[var(--color-ink)] underline hover:text-[var(--color-accent)]">
          Sign in
        </Link>
      </p>
    </Card>
  );
}
