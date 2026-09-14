import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';

// React 19 only batches inside `act()` when it is told it is in a test
// environment; @testing-library/react relies on this flag.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
});
