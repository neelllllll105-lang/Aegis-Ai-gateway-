# Design system — "Deskwork"

> Applies to `apps/web`. This is what shipped, not an aspiration — if something here
> stops matching `app/globals.css` or `components/ui.tsx`, fix this file or fix the code,
> don't let them drift apart.

## The idea

A warm, light canvas — the desk (`--color-bg`) — is the outer chrome: page background,
nav, footer, sidebar. Paper (`--color-surface` / `--color-surface2`), a shade lighter
than the desk, is where content lives — cards, tables, forms, the routing simulator.
Everything on paper is bordered in ink with a hard, zero-blur offset shadow, like
something physically sitting on the desk rather than a `box-shadow: 0 4px 6px
rgba(0,0,0,.1)` floating in space. Both desk and paper are light — this is a warm
whiteboard-and-sticky-note world, not a dark-mode chrome. (An earlier pass in this same
session inverted that — invented a dark outer chrome that the actual reference never had,
after the source spec had scrolled out of context. Fixed same-day; see the Session 10
correction entry in `MEMORY.md` if this comes up again.)

Ink is the default color for everything — text, borders, icons. **Red is the only event
color.** It means the agent (Aegis's routing engine acted), primary action, or danger.
Nothing else gets to be that loud. If you reach for red and it isn't one of those three
things, use a different token.

## Tokens (`app/globals.css`, `@theme` block)

All existing token *names* were kept — only values changed — so `scripts/check-design-tokens.mjs`
(which asserts every `var(--color-*)` reference resolves to something declared) needed no
changes and nothing else in the app had to be touched to pick up the new palette.

| Token | Role |
|---|---|
| `--color-bg` | The desk. Light warm canvas, outer chrome only — a shade darker/grayer than the paper cards on it, never a dark chrome. |
| `--color-desk-raised` / `--color-desk-line` | A raised panel on the desk (sidebar, footer) and its border. |
| `--color-surface` / `--color-surface2` | Paper. Primary and secondary content surfaces. |
| `--color-ink` | Default text and structural borders, on paper. |
| `--color-muted` / `--color-muted-light` | Body / tertiary text, on paper. |
| `--color-paper-on-desk` / `--color-muted-on-desk` / `--color-faint-on-desk` | The *same three roles*, re-pointed for text sitting directly on the desk (headers, nav, section titles with no card behind them) rather than on a paper card. On today's light desk these resolve to the same values as ink/muted/muted-light — the separate token names exist so a future dark-desk variant is a one-place value change, not a find-and-replace across every page. |
| `--color-accent` (= `--color-red`) | The agent / primary action / danger. One hue, one meaning. |
| `--color-amber` | Pending, staged, needs attention. |
| `--color-positive` | A settled good outcome — savings realized, spend back to normal, a verdict. Not a general "success" color; don't reach for it for anything that isn't actually resolved. |
| `--color-ochre` | A third party — an upstream provider, something outside Aegis's own decision. |

### Gotcha: text color depends on what's behind it, not what the text says

Anything that sits directly on the desk with no paper card behind it should use the
`*-on-desk` token trio, not `--color-ink`/`--color-muted`/`--color-muted-light` — today
they resolve to the same values, but a bare section's text color should describe *what
it's sitting on*, not just borrow whatever token happened to look right at the time.
`app/(marketing)/page.tsx`'s bare section headers and every dashboard page's
`SectionHeader` (never wrapped in a card) are the canonical examples — check those first
if you add a new bare section, and see the `-on-desk` token entry above for why the
separate names exist even though the values currently match.

## Type — three voices, one already existing

- **Document** — Spectral (serif). Headings, section titles, the wordmark.
- **UI** — Space Grotesk. Chrome, labels, body copy, buttons. (Replaced Plus Jakarta Sans.)
- **Machine** — JetBrains Mono, unchanged. IDs, timestamps, money, logs, anything exact.
- **Human** — Caveat, loaded but intentionally not used as a system voice anywhere yet.
  Aegis's dashboard is operational/tabular, not document-and-annotation, so there wasn't
  a real, honest surface for a handwritten voice yet. It's wired up in `layout.tsx` for
  the day there is one (a founder note, a signed approval) — don't force it in before then.

## Shadow ladder and border grammar

Hard offset shadows only — `Npx Npx 0 var(--shadow-color)`, never a blur radius. `.shadow-hard-1`
through `-5` in `globals.css`; most of the app uses 2 and 3 (resting and hover/raised).
`Card`/`Stat`/`TableShell`/buttons in `components/ui.tsx` already carry the right one — reach
for those before writing a new inline shadow value.

Border style carries meaning, not just weight:
- solid = structure (default)
- dashed (`.border-pending`) = pending / staged / human-editable
- double (`.border-ceremonial`) = rare, ceremonial emphasis
- a colored left rail (`.rail-agent` / `.rail-pending` / `.rail-verdict` / `.rail-muted` / `.rail-other`) = outcome/severity on a log-like item (see `DecisionCard`)

## Radius discipline

Nothing rounder than 11px except pills and circles. `--radius-2xl` and `--radius-3xl` are
both capped at `11px` in the token table — Tailwind's `rounded-2xl`/`rounded-3xl` inherit
that automatically since Tailwind 4 reads its radius scale from these same `@theme` keys.
Don't introduce a new arbitrary `rounded-[Npx]` above that cap.

## Components ported from the source spec (`components/ui.tsx`)

Section 7 of the source design doc — the patterns for narrating agent activity — is the
part worth having regardless of what gets re-skinned later, because it's product logic
expressed as design, not decoration. What's implemented:

- **`Stamp`** — a rotated, bordered one-word chip for a routing reason or cache outcome.
  Border color always matches text color (`border-color: currentColor`), so one `tone`
  prop sets the whole thing. Used on the Requests page and the homepage routing simulator
  for `RoutingReason`/cache-outcome badges.
- **`AttributionChip`** — `you` / `agent` / `other`. This is the direct visual home for
  Aegis's `X-Aegis-Requested-Model` vs `X-Aegis-Served-By` header pair — the product's
  core "here's what you asked for, here's what actually ran" claim. Used on the Requests
  table and the routing simulator's live receipt.
- **`DecisionCard`** — actor → action → timestamp → value → reason, with a severity rail.
  Built to narrate one routing decision as a stream item; not yet wired into a page (the
  Requests table currently does the same job as rows). Reach for this if/when there's a
  single-request detail view that wants to show the full `explanation: Vec<String>` as a
  sequence rather than a flat table row.
- **`DisclosureMeter`** — solid fill + diagonal hatch for "committed vs. not yet settled."
  Used on the Budgets page for today's spend vs. the usual daily baseline (solid = normal
  range, hatch = the anomalous portion above it). Deliberately *not* used on the budget
  limit rows themselves — the API doesn't return current spend against a budget's limit,
  only the limit, and faking a percentage against data that isn't there would misrepresent
  what's real vs. illustrative.

### What was deliberately left out

The source spec's staged-approval blocks and mode grammar (MANUAL/APPROVAL/AUTONOMOUS)
are built for a product where a human and an agent are jointly authoring something and
handing off turns. Aegis's routing engine doesn't work that way — it's automatic
per-request routing, not a co-authoring loop — so porting those patterns in would imply
an interaction model the product doesn't have. Skipped on purpose, not missed.

## Re-skin discipline

If this ever needs to become a different product's palette, there are exactly three
values to change: `--color-accent`, `--color-bg` (the desk), and the handwriting face.
Everything else — the shadow ladder, the border grammar, the radius cap, the semantic
color contract (red/amber/positive/ochre), the tracking-vs-size scale on mono eyebrows —
stays fixed. That constraint is what keeps this reading as one coherent system instead of
drifting into generic SaaS a few projects from now.
