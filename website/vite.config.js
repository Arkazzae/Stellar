import { defineConfig } from 'vite';

export default defineConfig({
  // The logo, mascot, icons and fonts are read from the repository's branding
  // folder, which sits outside this project.
  server: { fs: { allow: ['..'] } },
  // three.js alone is about 600 kB minified, so the default 500 kB warning
  // would fire on every build.
  build: { chunkSizeWarningLimit: 800 },
});
