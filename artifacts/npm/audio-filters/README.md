# Stellarr audio filters

Copy the files under `dist/audio/` to `/audio/` on your web server. Speex, RNNoise and GTCRN each have a WebAssembly module and an AudioWorklet. `manifest.json` records their size, SHA-256, origin and license. Stellarr serves all files from the instance.
