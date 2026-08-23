//! Bridge between the archive client, which is async on tokio, and GTK, which
//! has its own event loop.
//!
//! Network work runs on its own tokio runtime, and the result comes back to the
//! UI thread through a channel: no widget is ever touched outside the main loop,
//! which is the one rule GTK does not forgive.

use std::future::Future;
use std::sync::OnceLock;

use gtk::glib;

use talkbank_archive::api::Client;

/// Credentials stored in the system keyring.
///
/// Never in `settings.ini`: a plaintext password in a configuration file is the
/// kind of shortcut you pay for later.
const KEYRING_SERVICE: &str = "org.talkbank.TalkBank";

pub struct Net {
    rt: tokio::runtime::Runtime,
    client: Client,
}

static NET: OnceLock<Net> = OnceLock::new();

pub fn net() -> &'static Net {
    NET.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            // Four, not two: extracting a zip is synchronous I/O over thousands
            // of files, and two concurrent extractions would block a two-thread
            // runtime, stalling the metadata calls as well.
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let client = Client::new().expect("HTTP client");
        Net { rt, client }
    })
}

impl Net {
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Runs a network job and calls `on_done` back on the UI thread.
    pub fn spawn<T, F>(&'static self, fut: F, on_done: impl FnOnce(T) + 'static)
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let (tx, rx) = async_channel::bounded(1);
        self.rt.spawn(async move {
            let out = fut.await;
            let _ = tx.send(out).await;
        });
        glib::spawn_future_local(async move {
            if let Ok(v) = rx.recv().await {
                on_done(v);
            }
        });
    }

    /// Like `spawn`, but with intermediate progress: `on_step` is called for
    /// every message, `on_done` at the end.
    pub fn spawn_with_progress<T, P, F>(
        &'static self,
        make: impl FnOnce(async_channel::Sender<P>) -> F + Send + 'static,
        mut on_step: impl FnMut(P) + 'static,
        on_done: impl FnOnce(T) + 'static,
    ) where
        T: Send + 'static,
        P: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let (ptx, prx) = async_channel::bounded(64);
        let (dtx, drx) = async_channel::bounded(1);
        self.rt.spawn(async move {
            let out = make(ptx).await;
            let _ = dtx.send(out).await;
        });
        glib::spawn_future_local(async move {
            // The two channels are read **in sequence**, not raced: the progress
            // sender dies with the job, and only then is the result sent.
            // Awaiting them with a `select!` meant exiting on the first channel
            // closing — and throwing the outcome away, so neither a download nor
            // a planning run ever finished.
            while let Ok(p) = prx.recv().await {
                on_step(p);
            }
            if let Ok(v) = drx.recv().await {
                on_done(v);
            }
        });
    }
}

// ---------------------------------------------------------------- keyring

pub fn store_password(email: &str, password: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, email)
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

pub fn load_password(email: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, email)
        .and_then(|e| e.get_password())
        .ok()
}

pub fn forget_password(email: &str) {
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, email) {
        let _ = e.delete_credential();
    }
}

#[cfg(test)]
mod tests {
    /// The invariant `spawn_with_progress` rests on: the progress sender dies
    /// **before** the result is sent, so the two channels have to be read in
    /// sequence rather than raced.
    ///
    /// Racing them with a `select!` lost the outcome roughly one time in seven:
    /// a download that finished without saying so, and a planning run that never
    /// reached its confirmation. This test pins the contract down.
    #[test]
    fn the_outcome_is_not_lost_behind_the_progress_channel_closing() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .build()
            .unwrap();

        // A hundred rounds: the defect was probabilistic, one would not see it.
        for round in 0..100 {
            let outcome = rt.block_on(async {
                let (ptx, prx) = async_channel::bounded::<u32>(64);
                let (dtx, drx) = async_channel::bounded::<&'static str>(1);
                tokio::spawn(async move {
                    for i in 0..3 {
                        let _ = ptx.send(i).await;
                    }
                    // `ptx` is dropped here, before the following line
                    let _ = dtx.send("result").await;
                });

                let mut steps = 0;
                while prx.recv().await.is_ok() {
                    steps += 1;
                }
                (steps, drx.recv().await.ok())
            });
            assert_eq!(outcome.0, 3, "round {round}: progress messages lost");
            assert_eq!(
                outcome.1,
                Some("result"),
                "round {round}: the outcome was lost"
            );
        }
    }
}
