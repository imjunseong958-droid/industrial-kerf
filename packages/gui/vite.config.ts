import { defineConfig, loadEnv, type PluginOption } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "VITE_");
  // Browser builds remain same-origin. Desktop builds use the one loopback
  // host managed by Tauri, so the UI and HTTP host keep a single contract.
  const apiRoot = env.VITE_CUT_API_URL ?? (mode === "desktop" ? "http://127.0.0.1:4312" : "");
  return {
    // The monorepo also uses Vite through Vitest. Keep the GUI package
    // self-contained while accepting the plugin's compatible peer type when
    // npm happens to retain a second Vite copy in the workspace.
    plugins: [react() as unknown as PluginOption],
    define: {
      "import.meta.env.VITE_CUT_API_URL": JSON.stringify(apiRoot),
    },
    server: {
      port: 4174,
    },
  };
});
