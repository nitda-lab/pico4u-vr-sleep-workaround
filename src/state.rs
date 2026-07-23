use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

// Shared state for the background task
pub struct AppState {
    pub keep_awake_task: Mutex<Option<JoinHandle<()>>>,
    pub dim_task: Mutex<Option<JoinHandle<()>>>,
    pub is_running: Arc<AtomicBool>,
    pub debug_mode: Arc<AtomicBool>,
    pub adb_started_by_us: Arc<AtomicBool>,
    pub mic_drainer_task: Mutex<Option<JoinHandle<()>>>,
    pub mic_drainer_enabled: Arc<AtomicBool>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            keep_awake_task: Mutex::new(None),
            dim_task: Mutex::new(None),
            is_running: Arc::new(AtomicBool::new(false)),
            debug_mode: Arc::new(AtomicBool::new(false)),
            adb_started_by_us: Arc::new(AtomicBool::new(false)),
            mic_drainer_task: Mutex::new(None),
            mic_drainer_enabled: Arc::new(AtomicBool::new(false)),
        }
    }
}
