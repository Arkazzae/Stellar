# Localization

Stellarr supports English (`en`, default) and Polish (`pl`) across the web client, the Cockpit operator panel, and the ASP.NET Core API, plus optional in-browser message translation powered by Bergamot WASM.

## Locale resolution and `STELLARR_LANGUAGE`

Interface language and chat message translation are independent settings:

- **Interface language (`locale`)**: Controls UI copy and `Intl` date/time/number formatting.
- **Chat translation**: Optional client-side translation of chat messages into a chosen target language (`pl` or `en`); never alters stored message content.

Setting `STELLARR_LANGUAGE=pl` (or `en`) configures the instance default:

1. **Web client (`web/`)**: The server resolves the locale of every request ([`web/src/stellarr/i18n/requestLocale.ts`](../web/src/stellarr/i18n/requestLocale.ts)), in order:
   1. `stellarr_locale` cookie (written only when a user explicitly selects a language in Settings).
   2. `STELLARR_LANGUAGE` environment variable (read dynamically per request).
   3. `en`.

   The page arrives in that language with `<html lang>` set, and the language provider starts in it, so the first paint has its words and hydration matches. A changed `STELLARR_LANGUAGE` applies from the next request after a restart.
2. **Cockpit (`web/src/cockpit/`)**: Uses the language chosen in its own settings, kept in this browser's `localStorage` (`stellarr.cockpit.locale`), else `en`. It follows neither the browser's languages nor the web client's cookie. Because only the browser knows that choice, Cockpit renders in the browser: its routes send the document without the panel, and the panel appears in its language once the page has loaded.
3. **API (`server/Stellarr.Api/`)**: Reads `Stellarr:Language` (`STELLARR_LANGUAGE`) at startup (`LanguageOptions`). Used for API problem details, validation errors, system join notices, initial role/channel names on new servers, audit log entries, and password-reset/test emails.

> [!NOTE]
> Changing `STELLARR_LANGUAGE` affects server responses, emails, and database records created from that point forward. Records already persisted in SQLite keep the language in which they were originally written.

## Message catalogs

### Web client (`web/src/stellarr/i18n/`)

- [web/src/stellarr/i18n/messages/pl.ts](../web/src/stellarr/i18n/messages/pl.ts) defines the `MessageKey` union.
- [web/src/stellarr/i18n/messages/en.ts](../web/src/stellarr/i18n/messages/en.ts) is typed with `satisfies Record<keyof typeof pl, string>`, so missing or extra keys fail `npm run typecheck`.
- Components format strings via `t(key, values)`. `{name}` placeholders are interpolated as plain text without HTML parsing; unprovided placeholders remain literal (`{name}`).
- `LanguageProvider` persists:
  - `stellarr_locale` cookie so SSR and client hydration render the same `<html lang>`.
  - `stellarr_language_settings` in `localStorage` (`locale`, `chatTranslationEnabled`, `chatAutoTranslateEnabled`, `chatTranslationTargetLanguage`).

### Cockpit (`web/src/cockpit/i18n/`)

- Catalogs live in `web/src/cockpit/i18n/en.ts` and `web/src/cockpit/i18n/pl.ts` with matching key sets and `{name}` placeholder interpolation via `t(key, values)`.

### Backend API (`server/Stellarr.Api/Localization/`)

- `MessageKey.cs` enumerates all server-facing text keys.
- `Messages.en.cs` and `Messages.pl.cs` implement exhaustive switch expressions over `MessageKey`, and `Messages.Get(key, ...)` interpolates `{name}` placeholders.

## Client-side chat translation

> [!TIP]
> Chat translation runs locally inside the user's browser using WebAssembly and a Web Worker. Message text is never sent to an external translation service, and downloaded models remain cached in IndexedDB across sessions.

1. Enabling chat translation checks the three model files (`model`, `lex`, `vocab`) for the language pair (`plen` or `enpl`) listed in `web/public/bergamot-models/registry.json`.
2. Missing model files are fetched from `/bergamot-models/<pair>/<file>` on the instance itself, in development and in the image alike. They are not fetched from a third-party service. The files live in `artifacts/files/`, are copied into `web/public/` by `artifacts/assemble.mjs`, and are served by the web client's Next.js server.
3. Downloaded files are stored in IndexedDB (`bergamot-models`, store `files`) only after full completion. The registry records each file's size and SHA-256, and a test keeps both in agreement with `artifacts/manifest.json`.
4. A Web Worker loads the local Bergamot WASM runtime (`/codecs/bergamot/`, assembled from `artifacts/files/codecs/bergamot/`) and model off the main thread.
5. `web/src/stellarr/lib/chatTranslation/detectLanguage.ts` decides whether a message is Polish, English or neither, on the main thread and with nothing to download: it scores character trigrams against profiles of the two interface catalogues (derived by `web/scripts/derive-language-profiles.mjs`), counts function words of Polish, English and the languages most easily taken for them, and looks at the letters and the script. Messages already in the target language bypass translation; a message it cannot place, because it is too short or in another language, is translated from the other chat language.
6. Translations are cached in memory (up to 1,000 entries) and in IndexedDB (`stellarr-chat-translations`, up to 5,000 entries, LRU-trimmed). Concurrent requests for the same text are deduplicated, and visible viewport messages are prioritized.

`artifacts/manifest.json` is the single source of origins for the prebuilt files: each entry records the served path, size, SHA-256, upstream project, exact upstream file or release and SPDX license expression. `artifacts/assemble.mjs` checks every byte against it before writing, fails the build on a missing or changed file, and writes through a temporary file and rename so a bind-mounted development tree cannot expose a half-written worklet or WASM. `artifacts/NOTICES` records the copyright and license notices; the image carries both files under `/usr/share/doc/stellarr/artifacts/`. The worklets, WASM and models keep stable URLs across releases, so the server sends them with `Cache-Control: no-cache` (`headers()` in `web/next.config.ts`): the browser may keep a copy but must revalidate it, which keeps a worklet and the WASM it drives from different releases.

## Adding a new language

1. **Web client**:
   - Add the locale code to `SUPPORTED_LOCALES` in `web/src/stellarr/i18n/index.ts`, create `web/src/stellarr/i18n/messages/<code>.ts`, register it in `catalogs`, and add `language.name.<code>` to all catalogs.
   - Add the native language label to `NATIVE_NAMES` in `web/src/stellarr/components/account/panes/LanguagePane.tsx`.
   - Add the `Intl` BCP 47 tag to `LOCALE_TAG` in `web/src/stellarr/lib/format/format.ts` and verify plural rules (`pluralForm`).
   - If supporting local chat translation for the new language, update `TranslationTargetLanguage` in `web/src/stellarr/i18n/index.ts`, `primaryPair` in `web/src/stellarr/lib/chatTranslation/modelStatus.ts`, `artifacts/manifest.json`, `artifacts/NOTICES` and `web/public/bergamot-models/registry.json`. Put the model files under `artifacts/files/bergamot-models/<pair>/` (or add a recipe) and record their sizes and SHA-256 in both JSON files. `artifacts/assemble.mjs` then makes the served copies.
2. **Cockpit**:
   - Add `web/src/cockpit/i18n/<code>.ts` and register the locale in `web/src/cockpit/i18n/index.ts`.
3. **Backend API**:
   - Add the code to `LanguageOptions.cs`, add `Messages.<code>.cs`, and wire it into `Messages.cs`.

## Verification

Install dependencies once from the repository root with `npm ci`; `web/` (the web client and Cockpit) and `packages/` are workspaces sharing the root lockfile.

```bash
# Web client and Cockpit
cd web
npm test
npm run typecheck
npm run lint
npm run test:translation-runtime

# Cockpit's tests alone
npx vitest run --project cockpit

# Server localization tests
cd ..
dotnet test server/Stellarr.Api.Tests --filter LocalizationTests
```

`npm run test:translation-runtime` assembles the files from `artifacts/manifest.json` into `web/public/` and runs headless Chromium (`/usr/bin/chromium` or `STELLARR_CHROMIUM`) against the real Bergamot worker, WASM runtime and local model files. It translates a sample message and verifies that a second run loads the model from IndexedDB without downloading it again. It needs no network access and no running Stellarr.
