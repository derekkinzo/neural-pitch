//! Application state held as `tauri::State<AppState>`.
//!
//! `AppState` owns the lifecycle of the live capture pipeline:
//!
//! - `dsp` — `parking_lot::Mutex<Option<DspController>>` guarding the
//!   audio backend and DSP worker handle. `None` means "stopped".
//! - `store` — `tauri-plugin-store` handle obtained in `setup`, used by the
//!   settings commands to persist after a successful cache update.
//! - `settings` — in-memory cache of [`TunerSettings`]. Reads take the
//!   shared `RwLock`; writes take exclusive. `commands::set_setting` holds
//!   the write guard across the full read-modify-write so two concurrent
//!   patch calls cannot lose each other's deltas. The persist step runs
//!   *after* the guard is dropped because `parking_lot` guards are `!Send`
//!   and cannot cross `.await` (see `commands::persist_settings`).
//! - `events` — slot holding the most-recently-handed-out
//!   `tauri::ipc::Channel<AudioBackendEvent>`. Phase 1.3 wiring: the JS
//!   side constructs the channel once on mount and passes it into
//!   `start_capture`; the cpal backend's `err_fn` forwards
//!   `Disconnected` / `Underrun` / `FormatChanged` over that channel.
//!
//! `parking_lot::Mutex` is non-poisoning (ADR-0014). The atomic
//! "build_controller succeeds before the cache and disk are mutated" rule
//! lives in `commands::start_capture` — see that function for details.

use std::sync::Arc;
use std::thread::JoinHandle;

use neural_pitch_core::audio::{AudioBackend, AudioBackendEvent};
use neural_pitch_core::pipeline::DspError;
use neural_pitch_core::settings::TunerSettings;
use parking_lot::{Mutex, RwLock};
use tauri::Wry;
use tauri::ipc::Channel;
use tauri_plugin_store::Store;
use tokio_util::sync::CancellationToken;

/// Bundle of live capture machinery owned by [`AppState`] when capture is
/// running.
///
/// Dropping the backend during `stop_capture` tears the cpal `Stream` down,
/// satisfying the "stop frees the OS handle" rule from
/// [`neural_pitch_core::audio::backend`].
pub(crate) struct DspController {
    /// Concrete backend (cpal in production). Wrapping in `Box<dyn ...>`
    /// keeps the struct backend-agnostic so a future Phase 1.4 mock-driven
    /// integration test can swap it without rewriting the controller.
    pub(crate) backend: Box<dyn AudioBackend>,

    /// Join handle for the DSP worker thread. `None` while we are mid-
    /// `stop_capture` (after `take()`) so the helper can call `.join()` on
    /// the owned handle without holding the `AppState` mutex.
    pub(crate) worker_join: Option<JoinHandle<Result<(), DspError>>>,

    /// Cancellation token shared with the DSP worker. Calling `cancel()`
    /// from `stop_capture` causes the worker's loop to return on the next
    /// iteration.
    pub(crate) cancel: CancellationToken,
}

impl core::fmt::Debug for DspController {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DspController")
            .field("worker_alive", &self.worker_join.is_some())
            .field("cancelled", &self.cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// The single application-state struct injected via `tauri::Manager::manage`.
pub struct AppState {
    /// Lifecycle guard for the audio capture pipeline.
    pub(crate) dsp: Mutex<Option<DspController>>,

    /// Persistent settings store handle (`tauri-plugin-store`).
    pub(crate) store: Arc<Store<Wry>>,

    /// In-memory cache of [`TunerSettings`]. The store is the source of
    /// truth on disk; this cache keeps `get_settings` lock-free in the
    /// common (read) case.
    pub(crate) settings: RwLock<TunerSettings>,

    /// The most-recently-handed-out audio-event channel. The JS side
    /// constructs one [`Channel<AudioBackendEvent>`] on mount and passes it
    /// into `start_capture`. The cpal backend's `err_fn` forwards device
    /// events through it. Stored in an `Option` so a duplicate mount on
    /// the JS side replaces the prior channel without orphaning the
    /// previous handle (Tauri's `Channel` is reference-counted internally).
    pub(crate) events: Mutex<Option<Channel<AudioBackendEvent>>>,
}

impl AppState {
    /// Construct a new state with the supplied store handle and initial
    /// (already-validated) settings cache.
    pub fn new(store: Arc<Store<Wry>>, settings: TunerSettings) -> Self {
        Self {
            dsp: Mutex::new(None),
            store,
            settings: RwLock::new(settings),
            events: Mutex::new(None),
        }
    }

    /// Snapshot the current settings cache.
    pub(crate) fn settings_snapshot(&self) -> TunerSettings {
        self.settings.read().clone()
    }
}
