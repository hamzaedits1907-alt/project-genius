import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '../styles/global.css';
import '../styles/chat.css';
import '../styles/genius.css';
import '../styles/mobile.css';
import './web.css';
import WebApp from './WebApp';
import { RootBoundary } from './RootBoundary';
import { initMobileShell } from './mobileShell';

void initMobileShell();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RootBoundary>
      <WebApp />
    </RootBoundary>
  </StrictMode>,
);
