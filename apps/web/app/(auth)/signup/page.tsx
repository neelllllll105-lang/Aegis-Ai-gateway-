"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { api, ApiError } from "@/lib/api";
import { Button, Card, Field } from "@/components/ui";

/** Must match `MIN_PASSWORD_LENGTH` in the gateway. */
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

    // Check locally first so the user is not made to wait for a round trip to learn
    // something the form already knows.
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
    <Card className="p-6">
      <h1 className="text-lg font-medium text-[var(--color-ink)]">Create an account</h1>
      <p className="mt-1 text-sm text-[var(--color-ink-subtle)]">
        Free tier: 10,000 requests a month. No card required.
      </p>

      <form onSubmit={handleSubmit} className="mt-6 space-y-4">
        <Field
          label="Name"
          id="name"
          value={name}
          onChange={setName}
          autoComplete="name"
          placeholder="Optional"
        />
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
          autoComplete="new-password"
          required
          hint={
            passwordTooShort
              ? `${MIN_PASSWORD_LENGTH - password.length} more characters needed`
              : `At least ${MIN_PASSWORD_LENGTH} characters. A passphrase beats a short complex password.`
          }
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
          {submitting ? "Creating account…" : "Create account"}
        </Button>
      </form>

      <p className="mt-5 text-center text-sm text-[var(--color-ink-subtle)]">
        Already have an account?{" "}
        <Link href="/login" className="text-[var(--color-accent)] hover:underline">
          Sign in
        </Link>
      </p>
    </Card>
  );
}
