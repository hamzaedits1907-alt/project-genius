// Native behaviour of the Android / iOS apps: the system back button, returning to the app, the status bar and
// the on-screen keyboard. Does nothing in a browser.
import { App } from '@capacitor/app';
import { Keyboard } from '@capacitor/keyboard';
import { StatusBar, Style } from '@capacitor/status-bar';
import { useApp } from '../store/app';
import { isNativeApp } from './nativeKeys';

export async function initMobileShell(): Promise<void> {
  if (!isNativeApp()) return;
  StatusBar.setStyle({ style: Style.Dark }).catch(() => undefined);

  // Back: close the menu first, then go back to Chat, and only then leave the app.
  await App.addListener('backButton', () => {
    const st = useApp.getState();
    if (!st.sidebarCollapsed) return st.setSidebarCollapsed(true);
    if (st.route.name !== 'chat') return st.navigate({ name: 'chat' });
    void App.exitApp();
  });

  // Coming back to the app: pick up what changed on the other devices (the live refresh listens for this).
  await App.addListener('appStateChange', ({ isActive }) => {
    if (isActive) window.dispatchEvent(new Event('online'));
  });

  // Keyboard: the page is resized by the keyboard (capacitor.config), the safe-area padding under the message box goes away while it is open.
  await Keyboard.addListener('keyboardWillShow', () => document.body.classList.add('kb-open'));
  await Keyboard.addListener('keyboardWillHide', () => document.body.classList.remove('kb-open'));
}
