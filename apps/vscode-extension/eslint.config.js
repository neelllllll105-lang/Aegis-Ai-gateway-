// ESLint 9 flat config — the format the installed `eslint@^9.17.0` requires (the older
// `.eslintrc.*` format is no longer read at all as of v9). Kept intentionally small: this
// extension is six commands and one file, not a project that needs a large rule set.
const tseslint = require("@typescript-eslint/eslint-plugin");
const tsParser = require("@typescript-eslint/parser");

module.exports = [
  {
    files: ["src/**/*.ts"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        project: "./tsconfig.json",
        sourceType: "module",
      },
    },
    plugins: {
      "@typescript-eslint": tseslint,
    },
    rules: {
      ...tseslint.configs.recommended.rules,
      // The extension deliberately never lets a caught error pass silently past a user
      // message (every VS Code API call that can fail shows something) — this rule would
      // otherwise flag the intentional `catch {}` in URL validation as suspicious.
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
    },
  },
  {
    ignores: ["dist/**", "node_modules/**", "*.js"],
  },
];
