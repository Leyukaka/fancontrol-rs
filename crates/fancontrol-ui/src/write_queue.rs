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
}

enum WriteCmd {
    Set { id: String, percent: u8 },
}

impl WriteQueue {
    pub fn start(reg: Arc<ProviderRegistry>) -> Self {
        let (tx, rx) = mpsc::channel::<WriteCmd>();
        let last_error = Arc::new(Mutex::new(None));
        let err = Arc::clone(&last_error);
        thread::Builder::new()
            .name("fancontrol-write".into())
            .spawn(move || {
                let mut last_sent: std::collections::HashMap<String, (u8, Instant)> =
                    std::collections::HashMap::new();
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        WriteCmd::Set { id, percent } => {
                            // Coalesce: skip if same duty sent recently
                            if let Some((p, t)) = last_sent.get(&id) {
                                if *p == percent && t.elapsed() < Duration::from_millis(400) {
                                    continue;
                                }
                            }
                            match reg.set_duty(&ControlId::new(id.clone()), percent) {
                                Ok(()) => {
                                    last_sent.insert(id, (percent, Instant::now()));
                                    if let Ok(mut e) = err.lock() {
                                        *e = None;
                                    }
                                }
                                Err(e) => {
                                    if let Ok(mut g) = err.lock() {
                                        *g = Some(format!("{id}: {e}"));
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .expect("write thread");
        Self { tx, last_error }
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
}
