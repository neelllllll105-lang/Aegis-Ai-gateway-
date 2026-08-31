"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { api, ApiError } from "@/lib/api";
import { Button, Card, ErrorState, Field } from "@/components/ui";

export default function LoginPage() {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setSubmitting(true);

    try {
      await api.login(email, password);
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
      <h1 className="font-serif text-xl font-semibold tracking-tight text-[var(--color-ink)]">Sign in</h1>
      <p className="mt-1 text-xs text-[var(--color-muted)] font-medium">
        Welcome back to your Aegis dashboard.
      </p>

      <form onSubmit={handleSubmit} className="mt-6 space-y-4">
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
          autoComplete="current-password"
          required
        />

        {error && <ErrorState message={error} />}

        <Button type="submit" disabled={submitting} className="w-full mt-2">
          {submitting ? "Signing in…" : "Sign in to Dashboard"}
        </Button>
      </form>

      <p className="mt-6 text-center text-xs text-[var(--color-muted)] font-medium">
        Don&apos;t have an account?{" "}
        <Link href="/signup" className="font-bold text-[var(--color-ink)] underline hover:text-[var(--color-accent)]">
          Create free account
        </Link>
      </p>
    </Card>
  );
}
