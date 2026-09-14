/**
 * Rendering helpers. Screens expect the runtime and toast providers from
 * `App.tsx`; the tests render them the same way rather than mocking the hooks.
 */

import { render } from '@testing-library/react';
import type { ReactElement } from 'react';
import { RuntimeProvider } from '../hooks/useRuntime';
import { ToastProvider } from '../hooks/useToast';

export function renderWithProviders(ui: ReactElement) {
  return render(
    <ToastProvider>
      <RuntimeProvider>{ui}</RuntimeProvider>
    </ToastProvider>,
  );
}
