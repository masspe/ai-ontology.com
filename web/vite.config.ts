import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// API base URL the React app talks to. Override at build time with
// VITE_API_BASE — useful when serving the bundle from a different host
// than the ontology server.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
  },
});
