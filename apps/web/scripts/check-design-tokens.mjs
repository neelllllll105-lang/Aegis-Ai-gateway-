#!/usr/bin/env node
/**
 * Fail the build when a component references a design token that does not exist.
 *
 * # Why this check exists
 *
 * CSS custom properties fail silently. `color: var(--color-does-not-exist)` does not
 * error, does not warn, and does not show up in a type check or a lint run — the text
 * simply inherits and the page looks *almost* right. This repository shipped a palette
 * rename that left nine dashboard tokens dangling, and nothing caught it: `tsc` passed,
 * `eslint` passed, `next build` passed.
 *
 * So the check is mechanical. Read the tokens declared in `globals.css`, read every
 * `var(--color-*)` used in `app/` and `components/`, and assert the second set is a
 * subset of the first.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const GLOBALS = join(ROOT, "app", "globals.css");
const SOURCE_DIRS = [join(ROOT, "app"), join(ROOT, "components")];

/** Every `--color-*` declared at the start of a line in globals.css. */
function declaredTokens() {
  const css = readFileSync(GLOBALS, "utf8");
  return new Set([...css.matchAll(/^\s*(--color-[a-z0-9-]+)\s*:/gm)].map((m) => m[1]));
}

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      if (entry === "node_modules" || entry === ".next") continue;
      yield* walk(path);
    } else if (/\.(tsx|ts|css)$/.test(path) && path !== GLOBALS) {
      yield path;
    }
  }
}

const declared = declaredTokens();
const dangling = [];

for (const dir of SOURCE_DIRS) {
  for (const file of walk(dir)) {
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(/var\((--color-[a-z0-9-]+)\)/g)) {
      if (!declared.has(match[1])) {
        const line = source.slice(0, match.index).split("\n").length;
        dangling.push(`${relative(ROOT, file)}:${line}  ${match[1]}`);
      }
    }
  }
}

if (dangling.length > 0) {
  console.error(
    `\n${dangling.length} reference(s) to design tokens that are not declared in app/globals.css:\n`,
  );
  for (const entry of dangling) console.error(`  ${entry}`);
  console.error(
    "\nThese render as no colour at all. Either declare the token or use an existing one.\n",
  );
  process.exit(1);
}

console.log(`Design tokens OK — ${declared.size} declared, all references resolve.`);
