// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;

// Development only: when the page is opened in a normal browser (no Tauri around), install a
// fake backend with sample data so the interface can be previewed. In the real app Tauri has
// already provided `__TAURI_INTERNALS__`, and in a production build this branch is removed.
export async function load() {
  const w = typeof window === "undefined" ? undefined : (window as unknown as { __TAURI_INTERNALS__?: unknown });
  if (import.meta.env.DEV && w && !w.__TAURI_INTERNALS__) {
    await import("$lib/devmock");
  }
}
