"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { api, ApiError } from "@/lib/api";
import { Button, Card, Field } from "@/components/ui";

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
      // The gateway deliberately returns one message for every credential failure, so
      // this form cannot be used to discover which email addresses have accounts.
      setError(
        caught instanceof ApiError
          ? caught.message
          : "Could not reach the server. Check your connection and try again.",
      );
      setSubmitting(false);
    }
  }

  return (
    <Card className="p-6">
      <h1 className="text-lg font-medium text-[var(--color-ink)]">Sign in</h1>
      <p className="mt-1 text-sm text-[var(--color-ink-subtle)]">
        Welcome back.
      </p>

      <form onSubmit={handleSubmit} className="mt-6 space-y-4">
        <Field
          label="Email"
          id="email"
          type="email"
          value={email}
          onChange={setEmail}
          autoComplete="email"
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

        {error && (
          <div
            role="alert"
            className="rounded-[var(--radius)] border border-[#4a2424] bg-[#1a1112] px-3 py-2 text-sm text-[var(--color-danger)]"
          >
            {error}
          </div>
        )}

        <Button type="submit" disabled={submitting} className="w-full">
          {submitting ? "Signing in…" : "Sign in"}
        </Button>
      </form>

      <p className="mt-5 text-center text-sm text-[var(--color-ink-subtle)]">
        No account?{" "}
        <Link href="/signup" className="text-[var(--color-accent)] hover:underline">
          Create one
        </Link>
      </p>
    </Card>
  );
}
