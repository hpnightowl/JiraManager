// Tauri has no Node server, so this is a static single-page app.
export const ssr = false;

// Dev only: in a plain browser (no Tauri) load a fake backend with sample data to preview the UI.
export async function load() {
  const w = typeof window === "undefined" ? undefined : (window as unknown as { __TAURI_INTERNALS__?: unknown });
  if (import.meta.env.DEV && w && !w.__TAURI_INTERNALS__) {
    await import("$lib/devmock");
  }
}
