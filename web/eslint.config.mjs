import coreWebVitals from "eslint-config-next/core-web-vitals";
import typescript from "eslint-config-next/typescript";

// The flat exports of eslint-config-next are used directly. Going through
// FlatCompat re-serialises the Next plugin object, which contains a cycle, and
// ESLint throws before it lints anything.
const config = [
  {
    ignores: [
      ".next/**",
      "node_modules/**",
      "next-env.d.ts",
      // AudioWorklet modules are served verbatim as plain JavaScript, and
      // `allowJs` is off, so there is nothing here for the TypeScript rules.
      "public/**",
    ],
  },
  ...coreWebVitals,
  ...typescript,
  {
    rules: {
      "@typescript-eslint/consistent-type-imports": [
        "error",
        { prefer: "type-imports", fixStyle: "separate-type-imports" },
      ],
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // Images come from the server with unknown dimensions, and a self-hosted
      // instance has no image optimiser to hand them to.
      "@next/next/no-img-element": "off",
    },
  },
  {
    // Components are presentational: they take data as props and import types
    // from @/ports. Reaching past that into a transport is a lint error, since
    // a convention only holds until someone is in a hurry.
    files: ["src/components/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
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
                "Components are presentational. Import types from @/ports and take data as props; wire transports in src/containers or src/state.",
            },
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
    // routing from Next.js. These libraries were Cockpit's own copy of them;
    // a new import would bring that back.
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
            { name: "react-router-dom", message: "pages are routes under src/app/(cockpit)/cockpit/." },
          ],
          patterns: [
            { group: ["@radix-ui/*"], message: "UI parts come from @stellarr/ui." },
            {
              group: ["@/*"],
              message: "Cockpit does not use the web client's code; import its own modules by relative path.",
            },
          ],
        },
      ],
    },
  },
];

export default config;
