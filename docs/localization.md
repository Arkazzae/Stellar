# Localization

Stellarr supports English (`en`, default) and Polish (`pl`) across the web client, Stellarr Cockpit, and the ASP.NET Core API, plus optional offline in-browser chat translation powered by Bergamot WASM.

## Locale resolution and `STELLARR_LANGUAGE`

Interface language and chat message translation are configured independently:

- **Interface language (`locale`)**: Controls UI text and `Intl` date/time/number formatting.
- **Chat translation**: Optional client-side translation of messages into a target language (`en` or `pl`); never modifies stored message content.

Setting `STELLARR_LANGUAGE=pl` (or `en`) configures the instance default across all three components:

1. **Web client (`web/src/stellarr/`)**: Because the static client bundle is identical across deployments, container startup writes `STELLARR_LANGUAGE` to `/stellarr-instance.js` (`infra/entrypoint.sh`). A blocking script (`web/src/stellarr/i18n/localeScript.ts`) applies the first match onto `<html lang>` and `data-locale` before first paint:
   1. User preference saved in `localStorage` (`stellarr_language_settings.locale`, written only once the user explicitly picks a language in Settings)
   2. Instance default from `/stellarr-instance.js`
   3. `en`

   *(Browsers cache `/stellarr-instance.js` for up to 1 hour in production; dev/preview servers revalidate it on every load.)*
2. **Cockpit (`web/src/cockpit/`)**: Uses the locale saved in its own Settings page (`localStorage` key `stellarr.cockpit.locale`), defaulting to `en`. It ignores both browser preferences and the web client's locale, and does not load `/stellarr-instance.js`.
3. **API (`server/Stellarr.Api/`)**: Reads `Stellarr:Language` (`STELLARR_LANGUAGE`) at startup (`LanguageOptions`) for API problem details, validation errors, system join notices, default server/role/channel names, audit entries, and emails.

> [!NOTE]
> Changing `STELLARR_LANGUAGE` applies to API responses, emails, and new records going forward. Records already written to SQLite retain their original language.

## Message catalogs

- **Web client (`web/src/stellarr/i18n/`)**: [`messages/pl.ts`](../web/src/stellarr/i18n/messages/pl.ts) defines `MessageKey`, and [`messages/en.ts`](../web/src/stellarr/i18n/messages/en.ts) checks parity via `satisfies Record<keyof typeof pl, string>` at `npm run typecheck`. `t(key, values)` interpolates `{name}` placeholders as plain text. `LanguageProvider` persists `stellarr_language_settings` in `localStorage`.
- **Cockpit (`web/src/cockpit/i18n/`)**: `en.ts` and `pl.ts` enforce matching key sets and `{name}` interpolation.
- **Backend API (`server/Stellarr.Api/Localization/`)**: `MessageKey.cs` enumerates server message keys; `Messages.en.cs` and `Messages.pl.cs` implement exhaustive switch expressions over `MessageKey`.

## Client-side chat translation

> [!TIP]
> Chat translation runs locally inside the user's browser using WebAssembly and a Web Worker. Message text never leaves the browser, and downloaded models are cached in IndexedDB across sessions.

1. **Registry check**: Enabling translation checks the required `model`, `lex`, and `vocab` files for the pair (`plen` or `enpl`) in `web/public/bergamot-models/registry.json`.
2. **Same-origin download**: Missing model files download from `/bergamot-models/<pair>/<file>` on your Stellarr instance, never from an external host. Prebuilt assets live in `artifacts/files/` and are staged into `web/public/` by `artifacts/assemble.mjs`.
3. **IndexedDB caching**: Completed downloads are stored in IndexedDB (`bergamot-models`, store `files`) and verified against sizes and SHA-256 hashes shared with `artifacts/manifest.json`.
4. **Worker execution**: A Web Worker runs the Bergamot WASM runtime (`/codecs/bergamot/`) off the main thread.
5. **Language detection (`web/src/stellarr/lib/chatTranslation/detectLanguage.ts`)**: Runs on the main thread with zero extra downloads using character-trigram profiles (`web/scripts/derive-language-profiles.mjs`), function-word counts, and character/diacritic checks. Messages already in the target language are skipped; ambiguous short strings translate from the opposite language.
6. **Translation cache**: Results are cached in memory (up to 1,000 entries) and IndexedDB (`stellarr-chat-translations`, up to 5,000 entries, LRU-evicted), with request deduplication and viewport prioritization.

**Asset integrity and cache policy:** `artifacts/manifest.json` tracks the exact origin, size, SHA-256, and SPDX license for every prebuilt browser file (`artifacts/NOTICES` ships beside it at `/usr/share/doc/stellarr/artifacts/`). `artifacts/assemble.mjs` verifies every file before builds and replaces copies atomically. Because worklets, WASM, and models keep stable URLs across releases, Nginx serves them with `Cache-Control: no-cache` so browsers always revalidate matching versions together.

## Adding a new language

1. **Web client**:
   - Add the code to `SUPPORTED_LOCALES` in `web/src/stellarr/i18n/index.ts`, create `web/src/stellarr/i18n/messages/<code>.ts`, and add `language.name.<code>` across all catalogs.
   - Register the chunk in `LOADERS` (`web/src/stellarr/i18n/index.ts`), `CATALOGS` (`web/src/stellarr/i18n/localeScript.ts`), `CATALOG_MODULES`/`preloadPlan` (`web/documents.ts`), `CATALOG_UNAVAILABLE` (`web/src/stellarr/i18n/messages/unavailable.ts`), and `web/vitest.setup.ts` (`npm run typecheck` checks these).
   - Add the code to `write_instance_language` in `infra/entrypoint.sh` (checked by `web/documents.test.ts`).
   - Add the native label to `NATIVE_NAMES` in `web/src/stellarr/components/account/panes/LanguagePane.tsx` and the BCP 47 tag to `LOCALE_TAG` in `web/src/stellarr/lib/format/format.ts` (and verify `pluralForm`).
   - For chat translation support, update `TranslationTargetLanguage` (`i18n/index.ts`), `primaryPair` (`lib/chatTranslation/modelStatus.ts`), `artifacts/manifest.json`, `artifacts/NOTICES`, and `web/public/bergamot-models/registry.json`, placing model files under `artifacts/files/bergamot-models/<pair>/`.
2. **Cockpit**:
   - Add `web/src/cockpit/i18n/<code>.ts` and register the locale in `web/src/cockpit/i18n/index.ts`.
3. **Backend API**:
   - Add the code to `LanguageOptions.cs`, create `Messages.<code>.cs`, and wire it into `Messages.cs`.

## Verification

Install dependencies once from the repository root with `npm ci` (`web/` and `packages/` share the root workspace lockfile):

```bash
# Web client and Cockpit
cd web
npm test
npm run typecheck
npm run lint
npm run test:translation-runtime

# Cockpit tests alone
npx vitest run --project cockpit

# Server localization tests
cd ..
dotnet test server/Stellarr.Api.Tests --filter LocalizationTests
```

`npm run test:translation-runtime` stages assets from `artifacts/manifest.json` and runs headless Chromium (`/usr/bin/chromium` or `STELLARR_CHROMIUM`) offline against the real Bergamot worker and models, verifying both live translation and IndexedDB cache hits on reload.
