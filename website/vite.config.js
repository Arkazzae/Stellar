import { existsSync, readFileSync, statSync } from 'node:fs';
import { extname, resolve, sep } from 'node:path';
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

// index.html points at ../branding/ for the logo and the tour film. The build
// resolves those paths itself, but in the dev server the browser asks for them
// as /branding/..., outside the project root, so they are served from there.
const branding = fileURLToPath(new URL('../branding/', import.meta.url));
const brandingTypes = { '.svg': 'image/svg+xml', '.webp': 'image/webp', '.png': 'image/png', '.woff2': 'font/woff2' };

function brandingFiles() {
  return {
    name: 'stellarr-branding-files',
    apply: 'serve',
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = decodeURIComponent(request.url?.split('?')[0] ?? '');
        if (!path.startsWith('/branding/')) return next();
        const file = resolve(branding, path.slice('/branding/'.length));
        if (!file.startsWith(branding.endsWith(sep) ? branding : branding + sep)) return next();
        if (!existsSync(file) || !statSync(file).isFile()) return next();
        response.setHeader('Content-Type', brandingTypes[extname(file)] ?? 'application/octet-stream');
        response.end(readFileSync(file));
      });
    },
  };
}

export default defineConfig({
  // The logo, fonts and tour film are read from the repository's branding
  // folder, which sits outside this project.
  server: { fs: { allow: ['..'] } },
  plugins: [installScript(), brandingFiles()],
});
