# Localization

Stellarr supports English (`en`, default) and Polish (`pl`) across the web client, the Cockpit operator panel, and the ASP.NET Core API, plus optional in-browser message translation powered by Bergamot WASM.

## Locale resolution and `STELLARR_LANGUAGE`

Interface language and chat message translation are independent settings:

- **Interface language (`locale`)**: Controls UI copy and `Intl` date/time/number formatting.
- **Chat translation**: Optional client-side translation of chat messages into a chosen target language (`pl` or `en`); never alters stored message content.

Setting `STELLARR_LANGUAGE=pl` (or `en`) configures the instance default:

1. **Web client (`web/`)**: The client is a static export, the same files for every instance, so the language is chosen in the browser before the first paint. At every container start the entrypoint writes `STELLARR_LANGUAGE`, when it names a shipped language, to `/stellarr-instance.js`; the page loads that file as a blocking script, and an inline script ([`web/src/i18n/localeScript.tsx`](../web/src/i18n/localeScript.tsx)) then puts the first match of the following on `<html lang>` and `data-locale`:
   1. The language the person chose on this device (`locale` in `stellarr_language_settings` in `localStorage`).
   2. The `stellarr_locale` cookie, which remembers that choice and which Cockpit reads too.
   3. The instance default from `/stellarr-instance.js`.
   4. `en`.

   The pages are rendered ahead of time without any words: the HTML and the render that hydrates it hold only the shell's skeleton (on `/` and `/invite/<code>`) or the bare canvas, and `LocaleGate` renders the page right after hydration in the language on `<html lang>`. Nothing is shown in a wrong language and hydration cannot mismatch; the cost is that a first visit shows text only once the JavaScript has loaded. Browsers keep `/stellarr-instance.js` for up to an hour, so a changed `STELLARR_LANGUAGE` reaches a browser that loaded the client recently within that hour. `next dev` has no entrypoint and writes the same value inline from its own `STELLARR_LANGUAGE`.
2. **Cockpit (`cockpit/`)**: Resolves the locale in the browser on load (see [`cockpit/README.md`](../cockpit/README.md)):
   1. `stellarr_locale` cookie.
   2. First match in `navigator.languages` (`pl` or `en`).
   3. `en`.
3. **API (`server/Stellarr.Api/`)**: Reads `Stellarr:Language` (`STELLARR_LANGUAGE`) at startup (`LanguageOptions`). Used for API problem details, validation errors, system join notices, initial role/channel names on new servers, audit log entries, and password-reset/test emails.

> [!NOTE]
> Changing `STELLARR_LANGUAGE` affects server responses, emails, and database records created from that point forward. Records already persisted in SQLite keep the language in which they were originally written.

## Message catalogs

### Web client (`web/src/i18n/`)

- [web/src/i18n/messages/pl.ts](../web/src/i18n/messages/pl.ts) defines the `MessageKey` union.
- [web/src/i18n/messages/en.ts](../web/src/i18n/messages/en.ts) is typed with `satisfies Record<keyof typeof pl, string>`, so missing or extra keys fail `npm run typecheck`.
- Components format strings via `t(key, values)`. `{name}` placeholders are interpolated as plain text without HTML parsing; unprovided placeholders remain literal (`{name}`).
- `LanguageProvider` persists:
  - `stellarr_locale` cookie, read by the locale script and by Cockpit.
  - `stellarr_language_settings` in `localStorage` (`locale`, `chatTranslationEnabled`, `chatAutoTranslateEnabled`, `chatTranslationTargetLanguage`).

### Cockpit (`cockpit/src/i18n/`)

- Catalogs live in `cockpit/src/i18n/en.ts` and `cockpit/src/i18n/pl.ts` with matching key sets and `{name}` placeholder interpolation via `t(key, values)`.

### Backend API (`server/Stellarr.Api/Localization/`)

- `MessageKey.cs` enumerates all server-facing text keys.
- `Messages.en.cs` and `Messages.pl.cs` implement exhaustive switch expressions over `MessageKey`, and `Messages.Get(key, ...)` interpolates `{name}` placeholders.

## Client-side chat translation

> [!TIP]
> Chat translation runs locally inside the user's browser using WebAssembly and a Web Worker. Message text is never sent to an external translation service, and downloaded models remain cached in IndexedDB across sessions.

1. Enabling chat translation checks the three model files (`model`, `lex`, `vocab`) for the language pair (`plen` or `enpl`) listed in `web/public/bergamot-models/registry.json`.
2. Missing model files are fetched from `/bergamot-models/<pair>/<file>` on the instance itself, in development and in the image alike. They are not fetched from a third-party service. The files live in `artifacts/files/`, are copied into `web/public/` by `artifacts/assemble.mjs`, and are served from the static export.
3. Downloaded files are stored in IndexedDB (`bergamot-models`, store `files`) only after full completion. The registry records each file's size and SHA-256, and a test keeps both in agreement with `artifacts/manifest.json`.
4. A Web Worker loads the local Bergamot WASM runtime (`/codecs/bergamot/`, assembled from `artifacts/files/codecs/bergamot/`) and model off the main thread.
5. CLD3 detects the source message language; messages already in the target language bypass translation.
6. Translations are cached in memory (up to 1,000 entries) and in IndexedDB (`stellarr-chat-translations`, up to 5,000 entries, LRU-trimmed). Concurrent requests for the same text are deduplicated, and visible viewport messages are prioritized.

`artifacts/manifest.json` is the single source of origins for the prebuilt files: each entry records the served path, size, SHA-256, upstream project, exact upstream file or release and SPDX license expression. `artifacts/assemble.mjs` checks every byte against it before writing, fails the build on a missing or changed file, and writes through a temporary file and rename so a bind-mounted development tree cannot expose a half-written worklet or WASM. `artifacts/NOTICES` records the copyright and license notices; the image carries both files under `/usr/share/doc/stellarr/artifacts/`. The worklets, WASM and models keep stable URLs across releases, so Nginx serves them with `Cache-Control: no-cache`: the browser may keep a copy but must revalidate it, which keeps a worklet and the WASM it drives from different releases.

## Adding a new language

1. **Web client**:
   - Add the locale code to `SUPPORTED_LOCALES` in `web/src/i18n/index.ts`, create `web/src/i18n/messages/<code>.ts`, register it in `catalogs`, and add `language.name.<code>` to all catalogs.
   - Add the code to the languages `write_instance_language` accepts in `infra/entrypoint.sh`, so `STELLARR_LANGUAGE` can name it as the instance default.
   - Add the native language label to `NATIVE_NAMES` in `web/src/components/account/panes/LanguagePane.tsx`.
   - Add the `Intl` BCP 47 tag to `LOCALE_TAG` in `web/src/lib/format.ts` and verify plural rules (`pluralForm`).
   - If supporting local chat translation for the new language, update `TranslationTargetLanguage` in `web/src/i18n/index.ts`, `primaryPair` in `web/src/lib/chatTranslation/modelStatus.ts`, `artifacts/manifest.json`, `artifacts/NOTICES` and `web/public/bergamot-models/registry.json`. Put the model files under `artifacts/files/bergamot-models/<pair>/` (or add a recipe) and record their sizes and SHA-256 in both JSON files. `artifacts/assemble.mjs` then makes the served copies.
2. **Cockpit**:
   - Add `cockpit/src/i18n/<code>.ts` and register the locale in `cockpit/src/i18n/index.ts`.
3. **Backend API**:
   - Add the code to `LanguageOptions.cs`, add `Messages.<code>.cs`, and wire it into `Messages.cs`.

## Verification

Install dependencies once from the repository root with `npm ci`; `web/` and `cockpit/` are workspaces sharing the root lockfile.

```bash
# Web client
cd web
npm test
npm run typecheck
npm run lint
npm run test:translation-runtime

# Cockpit
cd ../cockpit
npm test
npm run typecheck

# Server localization tests
cd ..
dotnet test server/Stellarr.Api.Tests --filter LocalizationTests
```

`npm run test:translation-runtime` assembles the files from `artifacts/manifest.json` into `web/public/` and runs headless Chromium (`/usr/bin/chromium` or `STELLARR_CHROMIUM`) against the real Bergamot worker, WASM runtime and local model files. It translates a sample message and verifies that a second run loads the model from IndexedDB without downloading it again. It needs no network access and no running Stellarr.
