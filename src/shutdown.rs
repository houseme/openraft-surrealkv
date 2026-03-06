//! 优雅关闭信号管理

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 关闭信号
#[derive(Clone)]
pub struct ShutdownSignal {
    is_shutting_down: Arc<AtomicBool>,
}

impl ShutdownSignal {
    pub fn new() -> Self {
        Self {
            is_shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn shutdown(&self) {
        self.is_shutting_down.store(true, Ordering::SeqCst);
    }

    pub fn is_shutting_down(&self) -> bool {
        self.is_shutting_down.load(Ordering::SeqCst)
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}
