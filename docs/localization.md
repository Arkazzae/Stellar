# Localization

Stellarr supports English (`en`, default) and Polish (`pl`) across the web client, Stellarr Cockpit, and the Rust API, plus offline in-browser chat message translation powered by Mozilla's Bergamot WebAssembly engine.

## Locale resolution and `STELLARR_LANGUAGE`

Interface language and chat message translation are configured independently:

- **Interface language (`locale`)**: Controls UI copy and `Intl` date, time, and number formatting.
- **Chat translation**: Optional client-side translation of chat messages into a target language (`en` or `pl`); never alters stored message text.

Setting `STELLARR_LANGUAGE=pl` (or `en`) in `.env` configures the instance default across all three components:

1. **Web client (`web/src/stellarr/`)**: Because the static client bundle is identical across deployments, container startup (`infra/entrypoint.sh`) writes `STELLARR_LANGUAGE` to `/run/stellarr/stellarr-instance.js`, served by Nginx at `/stellarr-instance.js`. A blocking pre-paint script (`web/src/stellarr/i18n/localeScript.ts`) applies the resolved locale to `<html lang>` and `data-locale` before first paint, and inline preload scripts (`web/documents.ts`) `modulepreload` only the chosen language's catalog chunk:
   1. User preference saved in `localStorage` (`stellarr_language_settings.locale`, written only when the user explicitly selects a language in **Settings → Language**)
   2. Instance default from `/stellarr-instance.js` (cached for 1 hour by Nginx and served immediately with background revalidation by the Service Worker, plus a `localStorage` hint `stellarr_instance_language_hint` for early preloading)
   3. `en`
2. **Cockpit (`web/src/cockpit/`)**: Uses the locale chosen in its own **Settings** page (`localStorage` key `stellarr.cockpit.locale`), defaulting to `en`. It is independent of the web client's locale and does not load `/stellarr-instance.js`.
3. **API (`api/crates/server/`)**: Reads `Stellarr:Language` (`STELLARR_LANGUAGE`) at startup for API problem details, validation errors, system messages, default server/role/channel names, audit log sentences, and outgoing emails.

> [!NOTE]
> Changing `STELLARR_LANGUAGE` affects API responses, emails, and newly created database records going forward. Records already stored in SQLite retain the language they were written in.

## Message catalogs

- **Web client (`web/src/stellarr/i18n/`)**: [`messages/pl.ts`](../web/src/stellarr/i18n/messages/pl.ts) defines `MessageKey`, and [`messages/en.ts`](../web/src/stellarr/i18n/messages/en.ts) enforces key parity via `satisfies Record<keyof typeof pl, string>` at compile time. `t(key, values)` interpolates `{name}` placeholders as plain text. Each catalog compiles into a separate Rollup chunk (`web/documents.ts` fails the build if a catalog leaks into the main entry chunk or if the two catalogs share a chunk).
- **Cockpit (`web/src/cockpit/i18n/`)**: `en.ts` and `pl.ts` enforce matching key sets and `{name}` interpolation.
- **Backend API (`api/crates/messages/`)**: `key.rs` enumerates server message keys; `english.rs` and `polish.rs` implement exhaustive `match` expressions over `MessageKey`, so a key without a text in both languages does not compile.

## Client-side chat translation

> [!TIP]
> Chat translation runs entirely inside the user's browser using WebAssembly and a dedicated Web Worker (`web/public/bergamot-worker.js`). Message text never leaves the browser, and downloaded model files are cached in IndexedDB across sessions.

1. **Engine and models**:
   - The Bergamot WASM runtime (`/codecs/bergamot/bergamot-translator.wasm` and `bergamot-translator.js`, v0.6.0 from `mozilla/translations`) is compiled from source during the Docker build by [`artifacts/recipes/bergamot/`](../artifacts/recipes/README.md) (Emscripten 3.1.8 on `linux/amd64`, producing byte-identical WASM to Firefox Remote Settings).
   - Translation uses Mozilla's 2.x Marian models (`pl-en` 2.0 and `en-pl` 2.1: `model.<pair>.intgemm.alphas.bin`, `lex.50.50.<pair>.s2t.bin`, and shared `vocab.<pair>.spm`) listed in `web/public/bergamot-models/registry.json`.
2. **Same-origin download and SHA-256 verification**: When translation is enabled, missing model files are fetched from `/bergamot-models/<pair>/<file>` on the Stellarr instance (never from an external CDN) and verified against the exact byte size and SHA-256 digest in `registry.json` (which matches `artifacts/manifest.json`).
3. **Content-addressed IndexedDB model cache**: Verified model files are stored in IndexedDB (`bergamot-models`, object store `files`) **keyed by their SHA-256 digest**. Because `plen` and `enpl` share an identical SentencePiece vocabulary (`vocab.*.spm`), the vocabulary is downloaded and stored only once. On worker startup, `pruneStoredFiles` deletes any IndexedDB entries whose key is not in the current `registry.json`, automatically cleaning up models from older releases.
4. **Worker execution and sentence splitting**: `bergamot-worker.js` runs the WASM `BlockingService` off the main thread with a 40 MiB initial heap, passing language codes to `TranslationModel` so `Intl.Segmenter` splits multi-sentence messages accurately. If a single-sentence short reply (such as `"Dzięki!"` or `"No."`) is duplicated by the Marian model (`"Thanks! Thanks!"`), `withoutRepeat` collapses the duplicate sentence.
5. **Main-thread language detection (`web/src/stellarr/lib/chatTranslation/detectLanguage.ts`)**: Classifies messages synchronously with zero extra downloads using character-trigram profiles (`web/scripts/derive-language-profiles.mjs`), function-word tables, and diacritic checks. Messages already in the target language are skipped; short ambiguous strings translate from the opposite language.
6. **Translation result cache**: Translated messages are cached in memory (up to 1,000 entries) and in IndexedDB (`stellarr-chat-translations`, up to 5,000 entries with LRU eviction), with in-flight request deduplication and viewport prioritization.

**Asset integrity and cache headers:** `artifacts/manifest.json` records the upstream source, exact byte size, SHA-256 hash, and SPDX license for every prebuilt or recipe-built binary served to the browser (`artifacts/NOTICES` ships alongside it in the image at `/usr/share/doc/stellarr/artifacts/`). `artifacts/assemble.mjs` verifies every file before `dev`, `test`, and `build`. Because `/bergamot-worker.js`, `/codecs/bergamot/*`, and `/bergamot-models/*` use stable URLs across releases, Nginx serves them with `Cache-Control: no-cache` so browsers always revalidate matching versions together.

## Adding a new language

1. **Web client**:
   - Add the locale code to `SUPPORTED_LOCALES` in `web/src/stellarr/i18n/index.ts`, create `web/src/stellarr/i18n/messages/<code>.ts`, and add `language.name.<code>` across all catalogs.
   - Register the catalog chunk in `LOADERS` (`web/src/stellarr/i18n/index.ts`), `CATALOGS` (`web/src/stellarr/i18n/localeScript.ts`), `CATALOG_MODULES`/`preloadPlan` (`web/documents.ts`), `CATALOG_UNAVAILABLE` (`web/src/stellarr/i18n/messages/unavailable.ts`), and `web/vitest.setup.ts`.
   - Add the code to `write_instance_language` in `infra/entrypoint.sh` (enforced by `web/documents.test.ts`).
   - Add the native label to `NATIVE_NAMES` in `web/src/stellarr/components/account/panes/LanguagePane.tsx` and the BCP 47 tag to `LOCALE_TAG` in `web/src/stellarr/lib/format/format.ts` (and verify `pluralForm`).
   - For chat translation support, update `TranslationTargetLanguage` (`i18n/index.ts`), `primaryPair` (`lib/chatTranslation/modelStatus.ts`), `artifacts/manifest.json`, `artifacts/NOTICES`, and `web/public/bergamot-models/registry.json`, placing model files under `artifacts/files/bergamot-models/<pair>/`.
2. **Cockpit**:
   - Add `web/src/cockpit/i18n/<code>.ts` and register the locale in `web/src/cockpit/i18n/index.ts`.
3. **Backend API (`api/crates/messages/` & `api/crates/server/`)**:
   - Add the language variant to `Language` in `api/crates/messages/src/lib.rs`, create `api/crates/messages/src/<code>.rs` implementing every `MessageKey` match arm, and accept the new code in `STELLARR_LANGUAGE` validation (`api/crates/server/src/settings/variables.rs`).

## Verification

Install workspace dependencies from the repository root with `npm ci`, and build the recipe WebAssembly binaries ([`artifacts/recipes/`](../artifacts/recipes/README.md)) once with `docker buildx bake recipes`:

```bash
# Web client and Cockpit unit tests, typecheck, and lint
npm test -w stellarr-web
npm run typecheck -w stellarr-web
npm run lint -w stellarr-web

# Headless Chromium offline smoke test for the real Bergamot worker and models
npm run test:translation-runtime -w stellarr-web

# Cockpit tests alone
npx -w stellarr-web vitest run --project cockpit

# Server localization tests (run from api/)
cargo test -p stellarr-messages
```

`npm run test:translation-runtime -w stellarr-web` stages assets from `artifacts/manifest.json` and runs headless Chromium (`/usr/bin/chromium` or `STELLARR_CHROMIUM`) offline against `bergamot-worker.js` and the 2.x models, verifying `pl → en` and `en → pl` translation, `Intl.Segmenter` sentence splitting, duplicate short-reply collapsing, content-addressed IndexedDB caching across browser restarts, and pruning of stale model keys.
