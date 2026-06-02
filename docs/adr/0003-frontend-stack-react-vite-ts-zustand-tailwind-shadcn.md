# ADR-0003: Frontend stack: React 19 + Vite + TS strict + Zustand + Tailwind + shadcn/ui

## Status

Accepted — 2026-06-02.

## Context

The Tauri webview hosts a single-page application that owns the UI for the live tuner, recordings library, ear-training drills, and (eventually) song-analysis views. The choice of frontend stack drives a lot of downstream cost: AI-assist throughput, ecosystem maturity for component primitives, ease of typing IPC payloads, and (most importantly for the live tuner) the ability to bypass framework reactivity for the canvas hot path.

Candidates surveyed: SvelteKit, SolidJS, React 19. SvelteKit is appealing for its lightweight runtime; SolidJS offers fine-grained reactivity but a smaller ecosystem; React 19 has the largest mature component ecosystem and the strongest AI-assist coverage.

## Decision

The frontend stack is locked at:

- **React 19** as the framework.
- **Vite** (current LTS at design freeze) as the bundler.
- **TypeScript 5.x with `strict: true`**; `tsc --noEmit` runs in CI.
- **Zustand 5.x** for state management (lightweight; no Redux boilerplate).
- **Tailwind CSS 4.x** for utility-first styling.
- **shadcn/ui** (radix-ui primitives, copy-in pattern) for accessible component primitives.

The real-time canvas hot path **bypasses React reactivity**. Pitch frames are written into a `useRef`-held ring buffer; a `requestAnimationFrame` loop reads the ring and draws on `<canvas>` directly. State updates that would re-render the React tree are not used on this path.

TS-side type generation for Tauri command payloads uses `ts-rs` or `specta` (final pick at Phase 1 entry).

## Consequences

- AI-assist productivity is high; React 19 + shadcn/ui is the most heavily covered stack in current model training data.
- The shadcn copy-in pattern means there is no runtime dependency on `shadcn/ui` itself; primitives are vendored into `src/components/ui/`.
- The canvas-bypass pattern requires discipline: contributors who instinctively reach for `useState` on the live tuner page will degrade frame rate; this is documented in the frontend section of the design doc and reinforced in code review.
- `tsc --noEmit` failures are CI fails; no implicit-`any` workarounds.

## Alternatives Considered

- **SvelteKit** — rejected because shadcn/ui equivalent (`shadcn-svelte`) is less mature, and AI-assist coverage is materially weaker.
- **SolidJS** — rejected for ecosystem-size reasons; the canvas-bypass benefit it offers is already achievable in React via `useRef`.
- **Vue 3** — rejected for the same ecosystem-size reasons.
- **Pure web components** — rejected as a productivity step backward for a small team.
- **Redux / Redux Toolkit** — rejected as boilerplate-heavy for a project of this scope; Zustand covers the same need with ~5% the code.
