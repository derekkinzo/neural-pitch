// AudioBackendEvent — JSON shape emitted by the Rust cpal `err_fn` over
// the out-of-band `Channel<AudioBackendEvent>` passed into start_capture.
//
// Mirrors `crates/neural-pitch-core/src/audio/backend.rs::AudioBackendEvent`,
// which is serialised with `#[serde(tag = "kind", rename_all = "snake_case")]`.
//

import type { AudioBackendConfig } from "./audio-backend-config";

export type AudioBackendEvent =
  | { readonly kind: "disconnected" }
  | { readonly kind: "format_changed"; readonly new: AudioBackendConfig }
  | { readonly kind: "underrun"; readonly count: number };
