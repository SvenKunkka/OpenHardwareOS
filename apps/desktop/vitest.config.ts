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
  },
});
