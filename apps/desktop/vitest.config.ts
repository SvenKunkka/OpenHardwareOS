/// <reference types="vitest/config" />
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

/**
 * Frontend behaviour tests.
 *
 * They run against `src/lib/ipc.ts` mocked by the test (never the Tauri bridge),
 * in jsdom, and are excluded from the app bundle because nothing in `src`
 * imports them.
 */
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
    setupFiles: ['./src/test/setup.ts'],
    // Deterministic by default: no retries, no watch, no shared module state.
    clearMocks: false,
    restoreMocks: false,
    // These render real screens and drive real user actions in jsdom, and the
    // Windows CI runner needs 15-16 s for a file the development machine finishes
    // in under two. The default 5 s timeout turned that into four "tests failed"
    // that had nothing to do with the product. A timeout is still a real failure
    // here; it is just measured against a realistic bound.
    testTimeout: 30000,
    hookTimeout: 30000,
  },
});
