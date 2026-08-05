// Declared locally (not via the `vite/client` types) so this package doesn't need a `vite`
// dependency — it's meant to be reusable by the Phase 2 desktop web view later, not just Vite.
interface ImportMetaEnv {
  readonly VITE_GOOGLE_CLIENT_ID?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
