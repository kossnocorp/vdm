import cloudflare from "@astrojs/cloudflare";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://vdm.fyi",

  vite: {
    plugins: [tailwindcss()],
  },

  adapter: cloudflare(),
});
