import coreWebVitals from "eslint-config-next/core-web-vitals";
import typescript from "eslint-config-next/typescript";

// The web client and Cockpit share this workspace and nothing else, so no
// file of the web client's imports Cockpit's code, by alias or by path.
const notCockpit = {
  group: ["@cockpit", "@cockpit/*", "**/cockpit", "**/cockpit/**"],
  message: "The web client does not use Cockpit's code.",
};

// The flat exports of eslint-config-next are used directly. Going through
// FlatCompat re-serialises the Next plugin object, which contains a cycle, and
// ESLint throws before it lints anything.
const config = [
  {
    ignores: [
      ".next/**",
      "node_modules/**",
      "next-env.d.ts",
      "dist/**",
      ".bench-dist/**",
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
