"use client";

import { useEffect } from "react";

/**
 * The last resort — only reached when the root layout itself throws, which `error.tsx`
 * cannot catch (it renders *inside* the layout it's meant to protect). Next.js requires
 * this file to render its own complete `<html>`/`<body>`, replacing the root layout
 * entirely, so nothing here can depend on it: no `globals.css`, no next/font, no shared
 * component. Hardcoded, inline-styled, and deliberately boring — the one job this page
 * has is to never itself become the reason the page is blank.
 */
export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error(error);
  }, [error]);

  return (
    <html lang="en">
      <body
        style={{
          margin: 0,
          minHeight: "100vh",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          padding: "4rem 1.5rem",
          textAlign: "center",
          backgroundColor: "#211C14",
          color: "#F3EEDF",
          fontFamily:
            "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
        }}
      >
        <p
          style={{
            margin: 0,
            fontSize: "11px",
            fontWeight: 700,
            letterSpacing: "0.2em",
            textTransform: "uppercase",
            color: "#A8341E",
          }}
        >
          Aegis is unavailable
        </p>
        <h1 style={{ margin: "12px 0 0", fontSize: "28px", fontWeight: 600 }}>
          Something broke loading this page.
        </h1>
        <p style={{ margin: "16px 0 0", maxWidth: "28rem", fontSize: "14px", opacity: 0.8 }}>
          This is on us, not you. Reloading usually fixes it.
        </p>
        {error.digest && (
          <p style={{ margin: "12px 0 0", fontSize: "11px", opacity: 0.7 }}>
            Reference: {error.digest}
          </p>
        )}
        <button
          type="button"
          onClick={reset}
          style={{
            marginTop: "28px",
            padding: "10px 20px",
            borderRadius: "10px",
            border: "1.5px solid #F3EEDF",
            background: "transparent",
            color: "#F3EEDF",
            fontSize: "13px",
            fontWeight: 700,
            cursor: "pointer",
          }}
        >
          Reload
        </button>
      </body>
    </html>
  );
}
