import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './styles/global.css';
import './styles/chat.css';
import './styles/genius.css';
import './styles/mobile.css';
import { initDesktopAccount } from './web/desktopAccount';

initDesktopAccount();

// Block the webview's default context menu outside text fields (desktop-app feel).
document.addEventListener('contextmenu', (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest('input, textarea, .cm-editor, pre, .term')) e.preventDefault();
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
