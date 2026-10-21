import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// Vitest globals are off, so RTL's automatic cleanup does not register itself.
afterEach(cleanup);

// jsdom has no pointer capture; window dragging calls it.
Element.prototype.setPointerCapture ??= () => undefined;
Element.prototype.releasePointerCapture ??= () => undefined;

// jsdom has no canvas; React Bits' canvas effects check for a missing context and stay idle.
HTMLCanvasElement.prototype.getContext = (() => null) as never;
