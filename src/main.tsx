import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Desktop } from './shell/Desktop';
import { registerBuiltinApps } from './shell/builtinApps';
import './styles.css';

registerBuiltinApps();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Desktop />
  </StrictMode>,
);
