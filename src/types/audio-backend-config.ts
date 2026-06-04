// AudioBackendConfig — JSON shape mirroring
// `crates/neural-pitch-core/src/audio/backend.rs::AudioBackendConfig`.
//
// Used inside the `format_changed` variant of `AudioBackendEvent` so the
// front-end can update its negotiated-format readout when the cpal backend
// renegotiates with the device's default config (cpal #564 family).

export interface AudioBackendConfig {
  readonly sample_rate: number;
  readonly channels: number;
  readonly hop: number;
  readonly window: number;
}
