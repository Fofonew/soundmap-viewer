import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { setWorkerUrl } from 'maplibre-gl';
import 'maplibre-gl/dist/maplibre-gl.css';
// MapLibre v6 + Vite 8 (Rolldown) : l'URL du worker doit être fournie explicitement.
import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?url';
// Polices embarquées (l'app fonctionne hors ligne) : Barlow, inspirée de la
// signalétique routière, et JetBrains Mono pour les valeurs mesurées.
import '@fontsource/barlow/latin-400.css';
import '@fontsource/barlow/latin-500.css';
import '@fontsource/barlow/latin-600.css';
import '@fontsource/barlow-condensed/latin-500.css';
import '@fontsource/barlow-condensed/latin-600.css';
import '@fontsource/jetbrains-mono/latin-400.css';
import '@fontsource/jetbrains-mono/latin-500.css';
import './styles.css';
import { App } from './App';

setWorkerUrl(workerUrl);

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
