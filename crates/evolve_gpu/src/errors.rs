//! What happens when the graphics card or its driver reports an error (GPU_SAFETY.md, rules D1 to D3).
//!
//! By default wgpu turns an error it cannot hand to anyone into a panic. A game must not die because a driver
//! complained, so every device gets a handler that records the error and keeps going; the code that owns a view looks at
//! the log and switches to the CPU renderer when something is wrong. In development and in tests the same log makes the
//! test fail ([`ErrorLog::assert_clean`]), so validation errors are never silently ignored (rule D2).

use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Inner {
    errors: Vec<String>,
    lost: Option<String>,
}

/// The errors a device has reported and whether it has been lost. Cheap to clone; all clones share the same log.
#[derive(Clone, Default)]
pub struct ErrorLog(Arc<Mutex<Inner>>);

impl ErrorLog {
    /// Makes `device` report its errors and its loss here instead of panicking, and returns the log.
    pub fn install(device: &wgpu::Device) -> ErrorLog {
        let log = ErrorLog::default();
        let on_error = log.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            let message = error.to_string();
            eprintln!("evolve_gpu: the graphics card reported an error: {message}");
            on_error.record(message);
        }));
        let on_lost = log.clone();
        device.set_device_lost_callback(move |reason, message| {
            // Destroying the device on purpose (at the end of a test or the program) is not a loss.
            if reason != wgpu::DeviceLostReason::Destroyed {
                eprintln!("evolve_gpu: the graphics device was lost ({reason:?}): {message}");
                on_lost.record_loss(format!("{reason:?}: {message}"));
            }
        });
        log
    }

    pub fn record(&self, message: String) {
        self.0.lock().expect("the error log is never poisoned").errors.push(message);
    }

    pub fn record_loss(&self, message: String) {
        self.0.lock().expect("the error log is never poisoned").lost = Some(message);
    }

    /// Takes back a recorded loss (tests that pretend a device was lost on a device they share).
    pub fn clear_loss(&self) {
        self.0.lock().expect("the error log is never poisoned").lost = None;
    }

    /// The errors reported since the last call, oldest first.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().expect("the error log is never poisoned").errors)
    }

    /// Why the device was lost, if it was. The view that uses it must fall back to the CPU renderer.
    pub fn lost(&self) -> Option<String> {
        self.0.lock().expect("the error log is never poisoned").lost.clone()
    }

    /// True if the device is lost or has reported an error that has not been taken.
    pub fn has_problem(&self) -> bool {
        let inner = self.0.lock().expect("the error log is never poisoned");
        inner.lost.is_some() || !inner.errors.is_empty()
    }

    /// Fails (panics) if the device reported an error or was lost. For tests: the errors that used to panic inside wgpu
    /// are collected here instead, and checked at the end.
    pub fn assert_clean(&self) {
        let lost = self.lost();
        let errors = self.take();
        assert!(lost.is_none(), "the graphics device was lost: {}", lost.unwrap_or_default());
        assert!(errors.is_empty(), "the graphics card reported {} error(s):\n{}", errors.len(), errors.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Gpu;

    #[test]
    fn the_log_collects_and_hands_over_errors_once() {
        let log = ErrorLog::default();
        assert!(!log.has_problem());
        log.record("first".into());
        log.record("second".into());
        assert!(log.has_problem());
        assert_eq!(log.take(), ["first", "second"]);
        assert!(log.take().is_empty() && !log.has_problem());
        log.record_loss("driver reset".into());
        assert!(log.has_problem());
        assert_eq!(log.lost().as_deref(), Some("driver reset"));
    }

    #[test]
    #[should_panic(expected = "reported 1 error")]
    fn assert_clean_fails_when_something_was_reported() {
        let log = ErrorLog::default();
        log.record("validation: something is wrong".into());
        log.assert_clean();
    }

    /// A real validation error from a real device must land in the log (and not panic), which is what keeps a game alive
    /// when a driver complains, and what lets tests see it.
    #[test]
    fn a_real_validation_error_is_recorded_not_raised() {
        Gpu::for_each_test_mode(|gpu| {
            // A buffer with no usage flags is invalid.
            let _ = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("invalid"), size: 16, usage: wgpu::BufferUsages::empty(), mapped_at_creation: false });
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("the card finished");
            let errors = gpu.errors.take();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(gpu.errors.take().is_empty());
        });
    }
}
