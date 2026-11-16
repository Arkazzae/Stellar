import importPlugin from "eslint-plugin-import";
import jsxA11y from "eslint-plugin-jsx-a11y";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

/**
 * ESLint 10 took the old methods off a rule's context (`getFilename`,
 * `getSourceCode` and the like), and eslint-plugin-react and
 * eslint-plugin-import still call some of them. This hands each of their
 * rules a context that has them again, as ESLint 9 did.
 */
const patched = new WeakSet();
function withLegacyContext(plugin) {
  for (const rule of Object.values(plugin.rules ?? {})) {
    if (typeof rule !== "object" || patched.has(rule)) continue;
    patched.add(rule);
    const create = rule.create;
    rule.create = function (context) {
      if (typeof context.getFilename === "function") return create.call(this, context);
      const legacy = Object.freeze(
        Object.create(context, {
          getCwd: { value: () => context.cwd },
          getFilename: { value: () => context.filename },
          getPhysicalFilename: { value: () => context.physicalFilename },
          getSourceCode: { value: () => context.sourceCode },
          parserOptions: { value: { ...context.languageOptions.parserOptions } },
          parserPath: { value: undefined },
        }),
      );
      return create.call(this, legacy);
    };
  }
  return plugin;
}

// The web client and Cockpit share this workspace and nothing else, so no
// file of the web client's imports Cockpit's code, by alias or by path.
const notCockpit = {
  group: ["@cockpit", "@cockpit/*", "**/cockpit", "**/cockpit/**"],
  message: "The web client does not use Cockpit's code.",
};

const config = [
  {
    ignores: [
      "node_modules/**",
      "dist/**",
      ".bench-dist/**",
      // AudioWorklet modules are served verbatim as plain JavaScript, and
      // `allowJs` is off, so there is nothing here for the TypeScript rules.
      "public/**",
    ],
  },
  {
    // React, its hooks and accessibility: the recommended rules of each
    // plugin, with the adjustments for React 19 and TypeScript below.
    files: ["**/*.{js,jsx,mjs,ts,tsx,mts,cts}"],
    plugins: {
      react: withLegacyContext(react),
      "react-hooks": reactHooks,
      import: withLegacyContext(importPlugin),
      "jsx-a11y": jsxA11y,
    },
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
    settings: {
      react: { version: "detect" },
    },
    rules: {
      ...react.configs.recommended.rules,
      ...reactHooks.configs.recommended.rules,
      "import/no-anonymous-default-export": "warn",
      // TypeScript checks props and attributes, and the new JSX transform
      // needs no React in scope.
      "react/no-unknown-property": "off",
      "react/react-in-jsx-scope": "off",
      "react/prop-types": "off",
      "react/jsx-no-target-blank": "off",
      "jsx-a11y/alt-text": ["warn", { elements: ["img"], img: ["Image"] }],
      "jsx-a11y/aria-props": "warn",
      "jsx-a11y/aria-proptypes": "warn",
      "jsx-a11y/aria-unsupported-elements": "warn",
      "jsx-a11y/role-has-required-aria-props": "warn",
      "jsx-a11y/role-supports-aria-props": "warn",
    },
  },
  {
    files: ["**/*.{ts,tsx}"],
    plugins: { "@typescript-eslint": tseslint.plugin },
    languageOptions: {
      parser: tseslint.parser,
      parserOptions: { sourceType: "module" },
    },
  },
  ...tseslint.configs.recommended,
  {
    rules: {
      "@typescript-eslint/no-unused-expressions": "warn",
      "@typescript-eslint/consistent-type-imports": [
        "error",
        { prefer: "type-imports", fixStyle: "separate-type-imports" },
      ],
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
    },
  },
  {
    files: ["src/stellarr/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": ["error", { patterns: [notCockpit] }],
    },
  },
  {
    // Components are presentational: they take data as props and import types
    // from @/ports. Reaching past that into a transport, or into the router's
    // state, is a lint error, since a convention only holds until someone is
    // in a hurry. A component may render a `Link`; the pages read the address
    // and navigate. This replaces the rule above for components, so it bans
    // Cockpit's code as well.
    files: ["src/stellarr/components/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          paths: [
            {
              name: "@stellarr/router",
              importNames: ["Router", "Outlet", "useRouter", "usePathname", "useSearchParams", "useParams"],
              message:
                "Components are presentational. Take the address and navigation as props; the pages in src/stellarr/pages read the router.",
            },
          ],
          patterns: [
            {
              group: [
                "@/lib/api",
                "@/lib/api/*",
                "@/lib/realtime",
                "@/lib/realtime/*",
                "@/lib/voice",
                "@/lib/voice/*",
                "@/adapters",
                "@/adapters/*",
                "@/state",
                "@/state/*",
              ],
              // UI copy is not a transport, so the language context is the one
              // state import a component may read.
              allowImportNames: ["useLanguage"],
              message:
                "Components are presentational. Import types from @/ports and take data as props; wire transports in src/stellarr/containers or src/stellarr/state.",
            },
            notCockpit,
          ],
        },
      ],
    },
  },
  {
    // Cockpit, the operator panel, is an app of its own inside this workspace.
    // It reaches its own modules by relative path and none of the web
    // client's. Its UI parts come from @stellarr/ui, its icons from
    // src/cockpit/components/icons, which draws @stellarr/icons, and its
    // routing from @stellarr/router. These libraries were Cockpit's own copy
    // of them; a new import would bring that back.
    files: ["src/cockpit/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          paths: [
            { name: "lucide-react", message: "icons come from src/cockpit/components/icons." },
            { name: "class-variance-authority", message: "UI parts come from @stellarr/ui." },
            { name: "clsx", message: "join class names with cx from @stellarr/ui." },
            { name: "tailwind-merge", message: "join class names with cx from @stellarr/ui." },
            { name: "react-router-dom", message: "the addresses are in src/cockpit/routes.tsx, on @stellarr/router." },
          ],
          patterns: [
            { group: ["@radix-ui/*"], message: "UI parts come from @stellarr/ui." },
            {
              group: ["@/*", "**/stellarr", "**/stellarr/**"],
              message: "Cockpit does not use the web client's code; import its own modules by relative path.",
            },
          ],
        },
      ],
    },
  },
];

export default config;
