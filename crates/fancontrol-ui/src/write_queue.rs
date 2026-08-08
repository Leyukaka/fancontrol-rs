//! Background hardware write queue (keeps egui thread off EC sleeps).

use fancontrol_core::ControlId;
use fancontrol_plugins::ProviderRegistry;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub struct WriteQueue {
    tx: Sender<WriteCmd>,
    last_error: Arc<Mutex<Option<String>>>,
    /// Successful (id, duty) pairs since last poll by the UI.
    successes: Arc<Mutex<Vec<(String, u8)>>>,
    /// Control ids that failed since last poll (so UI can retry curve apply).
    failures: Arc<Mutex<Vec<String>>>,
}

enum WriteCmd {
    Set { id: String, percent: u8 },
}

impl WriteQueue {
    pub fn start(reg: Arc<ProviderRegistry>) -> Self {
        let (tx, rx) = mpsc::channel::<WriteCmd>();
        let last_error = Arc::new(Mutex::new(None));
        let successes = Arc::new(Mutex::new(Vec::new()));
        let failures = Arc::new(Mutex::new(Vec::new()));
        let err = Arc::clone(&last_error);
        let ok_log = Arc::clone(&successes);
        let fail_log = Arc::clone(&failures);
        thread::Builder::new()
            .name("fancontrol-write".into())
            .spawn(move || {
                let mut last_sent: std::collections::HashMap<String, (u8, Instant)> =
                    std::collections::HashMap::new();
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        WriteCmd::Set { id, percent } => {
                            // Coalesce: skip if same duty sent recently
                            if let Some((p, t)) = last_sent.get(&id)
                                && *p == percent
                                && t.elapsed() < Duration::from_millis(400)
                            {
                                // Still count as applied for UI skip-map (already on hardware)
                                if let Ok(mut s) = ok_log.lock() {
                                    s.push((id.clone(), percent));
                                }
                                continue;
                            }
                            match reg.set_duty(&ControlId::new(id.clone()), percent) {
                                Ok(()) => {
                                    last_sent.insert(id.clone(), (percent, Instant::now()));
                                    if let Ok(mut e) = err.lock() {
                                        *e = None;
                                    }
                                    if let Ok(mut s) = ok_log.lock() {
                                        s.push((id, percent));
                                    }
                                }
                                Err(e) => {
                                    if let Ok(mut g) = err.lock() {
                                        *g = Some(format!("{id}: {e}"));
                                    }
                                    if let Ok(mut f) = fail_log.lock() {
                                        f.push(id);
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .expect("write thread");
        Self {
            tx,
            last_error,
            successes,
            failures,
        }
    }

    pub fn enqueue(&self, id: &str, percent: u8) {
        let _ = self.tx.send(WriteCmd::Set {
            id: id.to_string(),
            percent,
        });
    }

    pub fn take_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|mut g| g.take())
    }

    pub fn take_successes(&self) -> Vec<(String, u8)> {
        self.successes
            .lock()
            .ok()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    }

    pub fn take_failures(&self) -> Vec<String> {
        self.failures
            .lock()
            .ok()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    }
}
