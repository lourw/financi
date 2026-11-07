//! One thread owns the SQLite connection. Callers send closures to it and
//! await the result, so the UI thread never blocks on disk.
//!
//! `rusqlite::Connection` is `Send` but not `Sync`; keeping it on a single
//! thread avoids a mutex and keeps writes serialized.

use std::sync::mpsc;
use std::thread;

use futures_channel::oneshot;
use rusqlite::Connection;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Handle to the database thread. Cheap to clone; the thread exits when the
/// last handle is dropped.
#[derive(Clone)]
pub struct DbWorker {
    jobs: mpsc::Sender<Job>,
}

/// The worker thread has exited (it panicked, or every handle was dropped).
#[derive(Debug, thiserror::Error)]
#[error("database worker is gone")]
pub struct WorkerGone;

impl DbWorker {
    /// Move `conn` onto a new thread named `financi-db`.
    pub fn spawn(conn: Connection) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("financi-db".into())
            .spawn(move || {
                let mut conn = conn;
                for job in rx {
                    // A panicking job drops its result sender (the caller sees
                    // WorkerGone) but must not take the database thread down.
                    let run = std::panic::AssertUnwindSafe(|| job(&mut conn));
                    if std::panic::catch_unwind(run).is_err() && !conn.is_autocommit() {
                        let _ = conn.execute_batch("ROLLBACK");
                    }
                }
            })
            .expect("failed to spawn financi-db thread");
        Self { jobs }
    }

    /// Run `f` on the database thread and await its result.
    pub async fn call<R, F>(&self, f: F) -> Result<R, WorkerGone>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> R + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.jobs
            .send(Box::new(move |conn| {
                let _ = tx.send(f(conn));
            }))
            .map_err(|_| WorkerGone)?;
        rx.await.map_err(|_| WorkerGone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_executor::block_on;

    #[test]
    fn runs_jobs_in_order_off_thread() {
        let worker = DbWorker::spawn(crate::open_in_memory().unwrap());
        let caller = thread::current().id();

        block_on(worker.call(|c| {
            c.execute("INSERT INTO settings (key, value) VALUES ('a', '1')", [])
                .unwrap()
        }))
        .unwrap();
        let (value, ran_on) = block_on(worker.call(|c| {
            let v: String = c
                .query_row("SELECT value FROM settings WHERE key = 'a'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            (v, thread::current().id())
        }))
        .unwrap();

        assert_eq!(value, "1");
        assert_ne!(ran_on, caller);
    }

    #[test]
    fn panicking_job_fails_alone_and_worker_survives() {
        let worker = DbWorker::spawn(crate::open_in_memory().unwrap());
        let res = block_on(worker.call(|c| -> () {
            c.execute_batch("BEGIN; INSERT INTO settings (key, value) VALUES ('x', '1');")
                .unwrap();
            panic!("boom")
        }));
        assert!(res.is_err());

        let rows: i64 = block_on(worker.call(|c| {
            c.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
                .unwrap()
        }))
        .expect("worker still serves jobs after a panic");
        assert_eq!(
            rows, 0,
            "the panicked job's open transaction was rolled back"
        );
    }
}
