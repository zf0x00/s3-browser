//! Upload/download jobs. Progress lives in atomics the I/O tasks bump; the UI
//! samples them a few times a second instead of being notified per chunk.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

pub struct Transfer {
    pub id: u64,
    pub direction: Direction,
    /// File name shown in the list.
    pub name: String,
    /// `bucket/key` on the S3 side.
    #[allow(dead_code)]
    pub remote: String,
    /// Local path.
    pub local: String,
    pub total: AtomicU64,
    pub done: AtomicU64,
    cancel: AtomicBool,
    status: Mutex<Status>,
    started: Mutex<Option<Instant>>,
    finished: Mutex<Option<Instant>>,
}

impl Transfer {
    pub fn new(id: u64, direction: Direction, name: String, remote: String, local: String) -> Arc<Self> {
        Arc::new(Self {
            id,
            direction,
            name,
            remote,
            local,
            total: AtomicU64::new(0),
            done: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            status: Mutex::new(Status::Queued),
            started: Mutex::new(None),
            finished: Mutex::new(None),
        })
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn set_running(&self) {
        *self.started.lock().unwrap() = Some(Instant::now());
        *self.status.lock().unwrap() = Status::Running;
    }

    pub fn finish(&self, result: &Result<()>) {
        *self.finished.lock().unwrap() = Some(Instant::now());
        *self.status.lock().unwrap() = match result {
            Ok(()) => Status::Done,
            Err(_) if self.cancel.load(Ordering::Relaxed) => Status::Cancelled,
            Err(e) => Status::Failed(e.to_string()),
        };
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        let mut s = self.status.lock().unwrap();
        if *s == Status::Queued {
            *s = Status::Cancelled;
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn check_cancel(&self) -> Result<()> {
        if self.is_cancelled() {
            bail!("cancelled");
        }
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status(), Status::Queued | Status::Running)
    }

    /// 0.0 – 1.0
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        let done = self.done.load(Ordering::Relaxed);
        match self.status() {
            Status::Done => 1.0,
            _ if total == 0 => 0.0,
            _ => (done as f64 / total as f64).min(1.0) as f32,
        }
    }

    /// Average bytes/second since start.
    pub fn speed(&self) -> f64 {
        let Some(start) = *self.started.lock().unwrap() else { return 0.0 };
        let end = self.finished.lock().unwrap().unwrap_or_else(Instant::now);
        let secs = end.duration_since(start).as_secs_f64().max(0.001);
        self.done.load(Ordering::Relaxed) as f64 / secs
    }
}
