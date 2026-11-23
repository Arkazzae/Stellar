import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

// The one-command installer is the repository's install.sh, served at
// /install.sh as it is, so the site never keeps a copy of its own.
const installer = fileURLToPath(new URL('../install.sh', import.meta.url));

function installScript() {
  return {
    name: 'stellarr-install-script',
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        if (request.url?.split('?')[0] !== '/install.sh') return next();
        response.setHeader('Content-Type', 'text/plain; charset=utf-8');
        response.end(readFileSync(installer));
      });
    },
    buildStart() {
      this.addWatchFile(installer);
    },
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'install.sh', source: readFileSync(installer) });
    },
  };
}

export default defineConfig({
  // The logo, fonts and tour film are read from the repository's branding
  // folder, which sits outside this project.
  server: { fs: { allow: ['..'] } },
  plugins: [installScript()],
});
