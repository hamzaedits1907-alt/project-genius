import type { CapacitorConfig } from '@capacitor/cli';

// Genius for Android and iOS: the web app packaged with Capacitor (build it with `npm run build:mobile`).
const config: CapacitorConfig = {
  appId: 'app.projectgenius.mobile',
  appName: 'Genius',
  webDir: 'dist-mobile',
  backgroundColor: '#0b0b12',
  server: { androidScheme: 'https' },
  android: { allowMixedContent: false },
  plugins: {
    Keyboard: { resize: 'body', resizeOnFullScreen: true },
    StatusBar: { style: 'DARK', backgroundColor: '#0b0b12', overlaysWebView: false },
  },
};

export default config;
