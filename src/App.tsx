import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export function App() {
  const [greeting, setGreeting] = useState<string>("Loading…");
  useEffect(() => {
    void invoke<string>("greet", { name: "Singer" }).then(setGreeting);
  }, []);
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-4 bg-slate-950 text-slate-50">
      <h1 className="text-4xl font-bold tracking-tight">NeuralPitch</h1>
      <p className="text-slate-300">Phase 0 — skeleton</p>
      <pre className="rounded bg-slate-900 px-4 py-2 text-sm text-slate-100">{greeting}</pre>
    </main>
  );
}
