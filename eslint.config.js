import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import importPlugin from "eslint-plugin-import";
import prettier from "eslint-config-prettier";
import globals from "globals";

// Web-standard globals allowed in packages/core and packages/storage — fetch + Web Crypto
// only, per AGENTS.md §4's "pure TS + web-standard APIs" boundary rule. Not window/document/etc.
const webStandardGlobals = {
  fetch: "readonly",
  Blob: "readonly",
  Response: "readonly",
  Request: "readonly",
  Headers: "readonly",
  crypto: "readonly",
  TextEncoder: "readonly",
  TextDecoder: "readonly",
  btoa: "readonly",
  atob: "readonly",
  structuredClone: "readonly",
};

export default tseslint.config(
  { ignores: ["**/dist/**", "**/node_modules/**", "**/.turbo/**", "**/coverage/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/consistent-type-imports": "error",
    },
  },
  // packages/core + packages/storage: platform-agnostic boundary (AGENTS.md §4), enforced,
  // not just documented.
  {
    files: ["packages/core/**/*.ts", "packages/storage/**/*.ts"],
    languageOptions: {
      globals: webStandardGlobals,
    },
    rules: {
      "no-restricted-globals": [
        "error",
        "window",
        "document",
        "navigator",
        "localStorage",
        "sessionStorage",
        "alert",
        "confirm",
        "prompt",
        "XMLHttpRequest",
        "WebSocket",
      ],
      "no-restricted-imports": [
        "error",
        {
          paths: ["react", "react-dom", "vite", "zustand"],
          patterns: [
            "node:*",
            "fs",
            "fs/*",
            "path",
            "os",
            "child_process",
            "@vrecnu/ui",
            "@vrecnu/publish",
            "@vrecnu/web",
            "@tauri-apps/*",
          ],
        },
      ],
    },
  },
  // packages/ui, packages/publish, packages/web: full browser environment + React rules.
  {
    files: ["packages/ui/**/*.{ts,tsx}", "packages/publish/**/*.{ts,tsx}", "packages/web/**/*.{ts,tsx}", "packages/desktop-ui/**/*.{ts,tsx}"],
    languageOptions: {
      globals: { ...globals.browser },
    },
    plugins: { "react-hooks": reactHooks, "react-refresh": reactRefresh, import: importPlugin },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
    },
  },
  // ADR-006: Tauri is confined to one crate on the Rust side and one package on the TS side.
  // This turns "no __TAURI__ branch in packages/publish" into a lint failure, not a review
  // comment. core/storage carry the same ban in their own block above, because a second
  // no-restricted-imports entry for those files would replace theirs rather than merge.
  {
    files: ["packages/**/*.{ts,tsx}"],
    ignores: ["packages/desktop-ui/**", "packages/core/**", "packages/storage/**"],
    rules: {
      "no-restricted-imports": [
        "error",
        { patterns: [{ group: ["@tauri-apps/*"], message: "Only packages/desktop-ui may import Tauri (ADR-006)." }] },
      ],
    },
  },
  // Dedicated Web Workers (OPFS writer, etc.) run in a worker global scope, not window.
  {
    files: ["**/*.worker.ts"],
    languageOptions: {
      globals: { ...globals.worker },
    },
  },
  // Node-executed tooling: config files and build scripts (icon generation, size gates).
  {
    files: ["*.config.{js,ts}", "**/*.config.{js,ts}", "**/scripts/**/*.{js,mjs,ts}"],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
  {
    files: ["**/*.test.{ts,tsx}", "**/__tests__/**/*.{ts,tsx}"],
    languageOptions: {
      globals: { ...globals.node },
    },
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
    },
  },
  prettier,
);
