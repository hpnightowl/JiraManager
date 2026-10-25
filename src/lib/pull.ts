import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PullProgress } from "./types";

/**
 * Download a model with your local Ollama. `onProgress` is called as it goes. Rejects with a
 * readable message if Ollama isn't running or the name is wrong. Safe to run again after an
 * interruption: Ollama continues where it stopped.
 */
export async function pullModel(model: string, onProgress: (p: PullProgress) => void): Promise<void> {
  const unlisten = await listen<PullProgress>("model-pull", (event) => onProgress(event.payload));
  try {
    await invoke("pull_model", { model });
  } finally {
    unlisten();
  }
}

/** 0-100 while the download of a file is running, otherwise null (manifest / verifying steps). */
export function percent(p: PullProgress | null): number | null {
  if (!p || p.total <= 0) return null;
  return Math.min(100, Math.floor((p.completed / p.total) * 100));
}

export function gigabytes(bytes: number): string {
  return (bytes / 1_000_000_000).toFixed(1);
}
