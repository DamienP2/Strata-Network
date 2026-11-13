import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const server = process.env.SN_SERVER ?? 'http://localhost:8080';

export default defineConfig({
  plugins: [react()],
  build: {
    assetsDir: 'static', // /assets est réservé aux fichiers servis par le serveur
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            { name: 'react', test: /node_modules[\\/](react|react-dom|scheduler|zustand|use-sync-external-store)[\\/]/ },
            { name: 'icons', test: /node_modules[\\/]lucide-react[\\/]/ },
          ],
        },
      },
    },
  },
  server: { port: 5173, proxy: { '/api': server, '/ws': { target: server.replace(/^http/, 'ws'), ws: true } } },
});
