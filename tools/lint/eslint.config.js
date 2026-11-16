// Undeclared-name check for the frontend modules (#764).
//
// Svelte components retain their compiler/type checks. Legacy browser modules
// also need no-undef: a typo in an unexecuted branch can otherwise pass a build.
// Keep this rule focused on correctness rather than introducing style churn.
import { fileURLToPath } from "node:url";
import globals from "globals";

// A relative basePath that leaves this folder matches nothing, so anchor the
// patterns at the repository root by absolute path.
const repoRoot = fileURLToPath(new URL("../..", import.meta.url));

export default [
  {
    basePath: repoRoot,
    files: ["app/ui/app.js", "app/ui/js/**/*.js"],
    languageOptions: {
      ecmaVersion: "latest",
      sourceType: "module",
      // Everything else the modules reach (the Tauri bridge, the mock) goes
      // through `window.`, so the standard browser globals are the whole list.
      globals: globals.browser,
    },
    rules: {
      "no-undef": "error",
    },
  },
];
