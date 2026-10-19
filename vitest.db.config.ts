import { defineConfig } from 'vitest/config';

// Database tests: real PostgreSQL (PGlite) with the real migrations. Run with `npm run test:db`.
export default defineConfig({ test: { environment: 'node', include: ['tests/db/**/*.test.ts', 'tests/netlify/**/*.test.ts'], testTimeout: 60000, hookTimeout: 60000 } });
