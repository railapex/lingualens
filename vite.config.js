import { defineConfig } from 'vite';
import { execFileSync } from 'child_process';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ttsCli = resolve(__dirname, 'src-tauri/target/debug/tts_cli.exe');

export default defineConfig({
  root: 'src',
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'src/index.html'),
        settings: resolve(__dirname, 'src/settings.html'),
        test: resolve(__dirname, 'src/test.html'),
      },
    },
  },
  server: {
    port: 1421,
    strictPort: true,
  },
  plugins: [{
    name: 'tts-api',
    configureServer(server) {
      server.middlewares.use('/api/phonemize', (req, res) => {
        const url = new URL(req.url, 'http://localhost');
        const text = url.searchParams.get('text');
        const lang = url.searchParams.get('lang') || 'es';
        if (!text) {
          res.writeHead(400);
          res.end(JSON.stringify({ error: 'missing text param' }));
          return;
        }
        try {
          const ipa = execFileSync(
            'C:/Program Files/eSpeak NG/espeak-ng.exe',
            ['-v', lang, '--ipa', '-q', text],
            { encoding: 'utf-8' },
          ).trim();
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ ipa }));
        } catch (e) {
          res.writeHead(500);
          res.end(JSON.stringify({ error: e.message }));
        }
      });

      server.middlewares.use('/api/speak', (req, res) => {
        const url = new URL(req.url, 'http://localhost');
        const text = url.searchParams.get('text');
        const lang = url.searchParams.get('lang') || 'es';
        const voice = url.searchParams.get('voice') || '';
        if (!text) {
          res.writeHead(400);
          res.end(JSON.stringify({ error: 'missing text param' }));
          return;
        }
        try {
          const args = [text, lang];
          if (voice) args.push(voice);
          const wav = execFileSync(ttsCli, args, { maxBuffer: 10 * 1024 * 1024 });
          res.writeHead(200, {
            'Content-Type': 'audio/wav',
            'Content-Length': wav.length,
          });
          res.end(wav);
        } catch (e) {
          res.writeHead(500);
          res.end(JSON.stringify({ error: e.message }));
        }
      });
    },
  }],
});
