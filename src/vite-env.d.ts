/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_SUPABASE_URL?: string;
  readonly VITE_SUPABASE_ANON_KEY?: string;
  /** Development only: run Genius Web without accounts (conversations stay in this browser). */
  readonly VITE_GENIUS_LOCAL?: string;
}
