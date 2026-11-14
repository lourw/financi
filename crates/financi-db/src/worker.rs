//! One thread owns the SQLite write connection, and — for file databases —
//! a second thread owns a read-only connection to the same file, so a slow
//! write (an import with pairing, a rule re-run) never blocks a dashboard
//! read: WAL allows reads while a write is in flight. Callers send closures
//! to a thread and await the result, so the UI thread never blocks on disk.
//!
//! `rusqlite::Connection` is `Send` but not `Sync`; keeping each connection
//! on a single thread avoids a mutex and keeps writes serialized.
//!
//! Keyed jobs ([`DbWorker::call_keyed`]) are latest-wins: at most one job
//! per key waits in the queue, and a job replaced by a newer one with the
//! same key resolves [`DbError::Superseded`]. Superseding a read that is
//! already running interrupts its query through the reader's
//! [`rusqlite::InterruptHandle`].

use std::collections::VecDeque;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use futures_channel::oneshot;
use rusqlite::{Connection, InterruptHandle, OpenFlags};

use crate::DbError;

/// How long either connection waits for a lock before failing (3.77).
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// The worker thread has exited (it panicked, or every handle was dropped).
#[derive(Debug, thiserror::Error)]
#[error("database worker is gone")]
pub struct WorkerGone;

/// One unit of work for a worker thread. The caller's result type is erased
/// into `run`. Keyed jobs also carry `supersede`, which resolves the caller
/// with `DbError::Superseded` without running `f`.
struct Job {
    label: String,
    key: Option<String>,
    queued_at: Instant,
    /// Set by `run` when the caller's closure panicked, so the thread can
    /// still roll back the connection's open transaction.
    panicked: Arc<AtomicBool>,
    run: Box<dyn FnOnce(&mut Connection) + Send>,
    supersede: Option<Box<dyn FnOnce() + Send>>,
}

/// The keyed job a worker thread is running, with its supersede callback,
/// so a newer job with the same key can resolve it and interrupt the query.
struct Running {
    key: String,
    supersede: Option<Box<dyn FnOnce() + Send>>,
}

#[derive(Default)]
struct Jobs {
    pending: VecDeque<Job>,
    running: Option<Running>,
    quit: bool,
}

/// The job queue of one worker thread: FIFO, with latest-wins keyed jobs.
struct Queue {
    jobs: Mutex<Jobs>,
    cv: Condvar,
    /// The thread's interrupt handle, when it has one (the reader), used to
    /// cut short a running keyed job that a newer job supersedes.
    interrupt: Option<Arc<InterruptHandle>>,
    /// Jobs this queue has accepted (`Queue::push` Ok paths). Counted for
    /// the 3.65 test hook, `DbWorker::job_count`.
    pushed: AtomicUsize,
}

impl Queue {
    fn new(interrupt: Option<Arc<InterruptHandle>>) -> Self {
        Self {
            jobs: Mutex::new(Jobs::default()),
            cv: Condvar::new(),
            interrupt,
            pushed: AtomicUsize::new(0),
        }
    }

    /// Enqueue `job`. A keyed job replaces the pending job with the same
    /// key (the old one resolves `Superseded`); if a job with that key is
    /// *running*, it resolves `Superseded` right away and the thread's
    /// query is interrupted when this queue has an interrupt handle.
    fn push(&self, job: Job) -> Result<(), WorkerGone> {
        let mut jobs = self.jobs.lock().unwrap();
        if jobs.quit {
            return Err(WorkerGone);
        }
        self.pushed.fetch_add(1, Ordering::Relaxed);
        if let Some(key) = job.key.as_deref() {
            let slot = jobs
                .pending
                .iter()
                .position(|j| j.key.as_deref() == Some(key));
            if let Some(ix) = slot {
                if let Some(supersede) = jobs.pending[ix].supersede.take() {
                    supersede();
                }
                // The new job takes the old one's place, keeping submit order.
                jobs.pending[ix] = job;
                self.cv.notify_one();
                return Ok(());
            }
            if jobs.running.as_ref().is_some_and(|r| r.key == key) {
                // Both are non-blocking: a oneshot send and setting SQLite's
                // interrupt flag, so it is fine to hold the queue lock.
                let running = jobs.running.take().expect("checked above");
                if let Some(supersede) = running.supersede {
                    supersede();
                }
                if let Some(handle) = &self.interrupt {
                    handle.interrupt();
                }
            }
        }
        jobs.pending.push_back(job);
        self.cv.notify_one();
        Ok(())
    }

    /// Stop the thread once the queue drains; queued jobs still run.
    fn quit(&self) {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.quit = true;
        self.cv.notify_all();
    }
}

/// Run queued jobs on `conn` until every `DbWorker` handle is dropped.
/// Pending jobs still run first, mirroring how an `mpsc` receiver drains.
fn serve(mut conn: Connection, queue: Arc<Queue>) {
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap();
            loop {
                if let Some(mut job) = jobs.pending.pop_front() {
                    if let Some(key) = &job.key {
                        jobs.running = Some(Running {
                            key: key.clone(),
                            supersede: job.supersede.take(),
                        });
                    }
                    break Some(job);
                }
                if jobs.quit {
                    break None;
                }
                jobs = queue.cv.wait(jobs).unwrap();
            }
        };
        let Some(job) = job else { return };

        let started = Instant::now();
        // A panicking job must not take the worker thread down. The result
        // senders already resolve the caller (dropped sender, or
        // `JobPanicked`); this keeps the thread alive for the next job.
        let panicked = catch_unwind(AssertUnwindSafe(|| (job.run)(&mut conn))).is_err();
        if (panicked || job.panicked.load(Ordering::Relaxed)) && !conn.is_autocommit() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        if job.key.is_some() {
            queue.jobs.lock().unwrap().running = None;
            // A supersede that raced the job finishing can leave SQLite's
            // interrupt flag set; consume it so it cannot fail the next job.
            if queue.interrupt.is_some() {
                let _ = conn.query_row("SELECT 1", [], |r| r.get::<_, i64>(0));
            }
        }
        if timings_enabled() {
            eprintln!(
                "[financi-db] {}: queue wait {:?}, run {:?}",
                job.label,
                started.duration_since(job.queued_at),
                started.elapsed()
            );
        }
    }
}

fn spawn_thread(name: &str, conn: Connection, queue: Arc<Queue>) {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || serve(conn, queue))
        .expect("failed to spawn the database thread");
}

/// With `FINANCI_TIMINGS=1` (debug builds), every job logs its label, queue
/// wait and run time on stderr. Tasks that want timing logs use this.
fn timings_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        cfg!(debug_assertions) && std::env::var_os("FINANCI_TIMINGS").is_some_and(|v| v == "1")
    })
}

/// Handle to the database threads. Cheap to clone; the threads exit when the
/// last handle is dropped.
#[derive(Clone)]
pub struct DbWorker {
    inner: Arc<Inner>,
}

struct Inner {
    writer: Arc<Queue>,
    /// The reader thread's queue, or `None` for a database that lives only
    /// in memory (a second connection could not see it); reads then run on
    /// the writer.
    reader: Option<Arc<Queue>>,
    /// Names recorded by [`QueryLog::record`] (3.78 test hook).
    #[cfg(debug_assertions)]
    queries: Arc<Mutex<Vec<String>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.writer.quit();
        if let Some(reader) = &self.reader {
            reader.quit();
        }
    }
}

/// Handle to the query log ([`DbWorker::query_log`], 3.78). Reload closures
/// call [`record`](Self::record) with each query's name as they run it;
/// tests read [`names`](Self::names) to assert what an action reloaded.
/// In release builds there is no log and `record` compiles to nothing.
#[derive(Clone)]
pub struct QueryLog {
    #[cfg(debug_assertions)]
    inner: Arc<Mutex<Vec<String>>>,
}

impl QueryLog {
    /// Note that a named query ran. Zero-cost in release builds.
    #[inline]
    pub fn record(&self, _name: &str) {
        #[cfg(debug_assertions)]
        self.inner.lock().unwrap().push(_name.to_string());
    }

    /// The names recorded so far, oldest first.
    #[cfg(debug_assertions)]
    pub fn names(&self) -> Vec<String> {
        self.inner.lock().unwrap().clone()
    }

    /// Forget everything recorded so far.
    #[cfg(debug_assertions)]
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
}

/// A job whose closure delivers the result itself; a panic drops the
/// sender, so the caller sees [`WorkerGone`].
fn plain_job<F>(label: &str, f: F) -> Job
where
    F: FnOnce(&mut Connection) + Send + 'static,
{
    Job {
        label: label.to_string(),
        key: None,
        queued_at: Instant::now(),
        panicked: Arc::new(AtomicBool::new(false)),
        run: Box::new(f),
        supersede: None,
    }
}

/// A typed job: `f`'s own [`DbError`] passes through, a panic resolves
/// [`DbError::JobPanicked`], and — for keyed jobs — a newer job with the
/// same key resolves the caller [`DbError::Superseded`] instead.
fn typed_job<R, F>(key: Option<String>, f: F, tx: oneshot::Sender<Result<R, DbError>>) -> Job
where
    R: Send + 'static,
    F: FnOnce(&mut Connection) -> Result<R, DbError> + Send + 'static,
{
    let label = match &key {
        Some(key) => key.clone(),
        None => "call".to_string(),
    };
    // Whoever resolves the caller first takes the sender; the other drops.
    let tx = Arc::new(Mutex::new(Some(tx)));
    let panicked = Arc::new(AtomicBool::new(false));

    let run_tx = Arc::clone(&tx);
    let run_panicked = Arc::clone(&panicked);
    let run = Box::new(move |conn: &mut Connection| {
        let result = catch_unwind(AssertUnwindSafe(|| f(conn)));
        if result.is_err() {
            run_panicked.store(true, Ordering::Relaxed);
        }
        if let Some(tx) = run_tx.lock().unwrap().take() {
            let _ = tx.send(result.unwrap_or(Err(DbError::JobPanicked)));
        }
    });
    let sup_tx = Arc::clone(&tx);
    let supersede = Box::new(move || {
        if let Some(tx) = sup_tx.lock().unwrap().take() {
            let _ = tx.send(Err(DbError::Superseded));
        }
    });
    Job {
        label,
        key,
        queued_at: Instant::now(),
        panicked,
        run,
        supersede: Some(supersede),
    }
}

impl DbWorker {
    /// Move `conn` onto a new thread named `financi-db`. When `conn` is a
    /// file database, also start a reader thread (`financi-db-read`) with
    /// its own read-only connection to the same file, so reads run during a
    /// write. In-memory databases get no reader; their reads queue on the
    /// writer. A reader that cannot be opened falls back the same way.
    pub fn spawn(conn: Connection) -> Self {
        let _ = conn.busy_timeout(BUSY_TIMEOUT);
        let reader_conn = conn
            .path()
            .filter(|p| !p.is_empty())
            .and_then(|path| {
                Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY
                        | OpenFlags::SQLITE_OPEN_URI
                        | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .ok()
            })
            .inspect(|reader| {
                let _ = reader.busy_timeout(BUSY_TIMEOUT);
            });

        let writer = Arc::new(Queue::new(None));
        spawn_thread("financi-db", conn, Arc::clone(&writer));
        let reader = reader_conn.map(|reader| {
            let queue = Arc::new(Queue::new(Some(Arc::new(reader.get_interrupt_handle()))));
            spawn_thread("financi-db-read", reader, Arc::clone(&queue));
            queue
        });

        Self {
            inner: Arc::new(Inner {
                writer,
                reader,
                #[cfg(debug_assertions)]
                queries: Arc::new(Mutex::new(Vec::new())),
            }),
        }
    }

    /// A handle to the query log (3.78): the names every reload records as
    /// its queries run. Tests use it to assert that an action reloaded
    /// exactly the data it should — and none of what it should not. The
    /// log is shared by every clone of this worker; in release builds the
    /// handle is empty and recording compiles to nothing.
    pub fn query_log(&self) -> QueryLog {
        QueryLog {
            #[cfg(debug_assertions)]
            inner: Arc::clone(&self.inner.queries),
        }
    }

    /// Jobs accepted by both queues since this worker started (3.65: lets
    /// a UI test count the queries one launch or click issues — exactly
    /// one refresh means exactly one job). Debug builds only.
    #[cfg(debug_assertions)]
    pub fn job_count(&self) -> usize {
        let count = |q: &Queue| q.pushed.load(Ordering::Relaxed);
        count(&self.inner.writer) + self.inner.reader.as_deref().map_or(0, count)
    }

    /// Run `f` on the writer thread and await its result. Writes and reads
    /// that must see a write made moments ago belong here; plain list and
    /// aggregate reads use [`call_read`](Self::call_read).
    pub async fn call<R, F>(&self, f: F) -> Result<R, WorkerGone>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> R + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.inner.writer.push(plain_job("call", move |conn| {
            let _ = tx.send(f(conn));
        }))?;
        rx.await.map_err(|_| WorkerGone)
    }

    /// Run `f` on the reader thread (on the writer when there is none) and
    /// await its result. The job is queued when this is called, not when
    /// the returned future is first polled; dropping the future does not
    /// cancel it.
    pub fn call_read<R, F>(&self, f: F) -> impl Future<Output = Result<R, WorkerGone>> + Send
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> R + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let submitted = self.read_queue().push(plain_job("read", move |conn| {
            let _ = tx.send(f(conn));
        }));
        async move {
            submitted?;
            rx.await.map_err(|_| WorkerGone)
        }
    }

    /// Queue a latest-wins job on the reader thread (on the writer when
    /// there is none). At most one job per `key` waits in the queue: a
    /// newer job replaces a pending one, and supersedes a running one
    /// immediately, interrupting its query. Either replaced job resolves
    /// [`DbError::Superseded`], which callers drop quietly. The job is
    /// queued when this is called, not when the returned future is first
    /// polled; dropping the future does not cancel it.
    pub fn call_keyed<R, F>(
        &self,
        key: &str,
        f: F,
    ) -> impl Future<Output = Result<R, DbError>> + Send
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R, DbError> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let submitted = self
            .read_queue()
            .push(typed_job(Some(key.to_string()), f, tx));
        async move {
            submitted?;
            rx.await.map_err(|_| DbError::WorkerGone(WorkerGone))?
        }
    }

    fn read_queue(&self) -> Arc<Queue> {
        self.inner
            .reader
            .clone()
            .unwrap_or_else(|| Arc::clone(&self.inner.writer))
    }
}

/// Run `f` on the writer thread, keeping the [`DbError`] instead of
/// flattening it, so callers can tell one failure from another. New code
/// prefers this over the app crate's string-flattening `on_db` helper.
pub async fn on_db_typed<R, F>(db: &DbWorker, f: F) -> Result<R, DbError>
where
    R: Send + 'static,
    F: FnOnce(&mut Connection) -> Result<R, DbError> + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    db.inner.writer.push(typed_job(None, f, tx))?;
    rx.await.map_err(|_| DbError::WorkerGone(WorkerGone))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_executor::block_on;
    use std::sync::atomic::AtomicUsize;

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

    /// A file database (so the reader thread starts) with its temp dir.
    fn open_temp_db() -> (Connection, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("financi-worker-test-{}", uuid::Uuid::new_v4()));
        let conn = crate::open(&dir.join("financi.sqlite")).unwrap();
        (conn, dir)
    }

    fn close_db(worker: DbWorker, dir: std::path::PathBuf) {
        // The threads exit on the last handle; give them a moment to close
        // the files before the temp dir goes away.
        drop(worker);
        thread::sleep(Duration::from_millis(100));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keyed_jobs_queued_behind_a_slow_one_run_only_the_last() {
        let worker = DbWorker::spawn(crate::open_in_memory().unwrap());
        let ran = Arc::new(AtomicUsize::new(0));

        let slow = worker.call_read(|_| thread::sleep(Duration::from_millis(150)));
        let mut keyed = Vec::new();
        for n in 1..=3 {
            let ran = Arc::clone(&ran);
            keyed.push(worker.call_keyed("refresh", move |_| {
                ran.fetch_add(1, Ordering::SeqCst);
                Ok(n)
            }));
        }

        let k1 = keyed.remove(0);
        let k2 = keyed.remove(0);
        let k3 = keyed.remove(0);
        assert!(matches!(block_on(k1), Err(DbError::Superseded)));
        assert!(matches!(block_on(k2), Err(DbError::Superseded)));
        assert_eq!(block_on(k3).unwrap(), 3);
        block_on(slow).unwrap();
        assert_eq!(ran.load(Ordering::SeqCst), 1, "only the last keyed job ran");
    }

    #[test]
    fn a_read_completes_while_a_slow_write_is_running() {
        let (conn, dir) = open_temp_db();
        let worker = DbWorker::spawn(conn);

        let slow_write = {
            let worker = worker.clone();
            thread::spawn(move || {
                block_on(worker.call(|c| {
                    c.execute_batch(
                        "BEGIN; INSERT INTO settings (key, value) VALUES ('slow', '1');",
                    )
                    .unwrap();
                    thread::sleep(Duration::from_millis(300));
                    c.execute_batch("COMMIT").unwrap();
                }))
            })
        };
        thread::sleep(Duration::from_millis(50)); // let the write get going

        let value = block_on(worker.call_read(|c| {
            c.query_row("SELECT value FROM settings WHERE key = 'slow'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
        }))
        .expect("the reader answers while the writer is busy");
        assert_eq!(
            value, None,
            "the reader is not blocked and sees only committed rows"
        );

        slow_write.join().unwrap().unwrap();
        let value = block_on(worker.call_read(|c| {
            c.query_row("SELECT value FROM settings WHERE key = 'slow'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
        }))
        .unwrap();
        assert_eq!(value.as_deref(), Some("1"), "the reader sees the commit");
        close_db(worker, dir);
    }

    #[test]
    fn superseding_a_running_read_interrupts_it() {
        let (conn, dir) = open_temp_db();
        let worker = DbWorker::spawn(conn);

        // A long recursive COUNT that SQLite can interrupt mid-flight.
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let stale = worker.call_keyed("refresh", move |c| {
            let _ = started_tx.send(());
            let n: i64 = c.query_row(
                "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 5000000) \
                 SELECT COUNT(*) FROM c",
                [],
                |r| r.get(0),
            )?;
            Ok(n)
        });
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the stale read is running");
        let fresh = worker.call_keyed("refresh", |c| {
            let n: i64 = c.query_row("SELECT 40", [], |r| r.get(0))?;
            Ok(n)
        });

        assert!(matches!(block_on(stale), Err(DbError::Superseded)));
        assert_eq!(
            block_on(fresh).unwrap(),
            40,
            "the reader serves the next job after an interrupt"
        );
        close_db(worker, dir);
    }

    #[test]
    fn the_reader_connection_refuses_writes() {
        let (conn, dir) = open_temp_db();
        let worker = DbWorker::spawn(conn);
        let wrote =
            block_on(worker.call_read(|c| {
                c.execute("INSERT INTO settings (key, value) VALUES ('a', '1')", [])
            }))
            .expect("the read job ran");
        assert!(wrote.is_err(), "the second connection is read-only");
        close_db(worker, dir);
    }

    #[test]
    fn panicking_typed_job_reports_job_panicked() {
        let worker = DbWorker::spawn(crate::open_in_memory().unwrap());
        let res = block_on(on_db_typed(&worker, |c| -> Result<(), DbError> {
            c.execute_batch("BEGIN; INSERT INTO settings (key, value) VALUES ('x', '1');")?;
            panic!("boom")
        }));
        assert!(matches!(res, Err(DbError::JobPanicked)));

        let rows = block_on(worker.call(|c| {
            c.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
                .unwrap()
        }))
        .expect("worker still serves jobs after a panic");
        assert_eq!(rows, 0, "the panicking job's transaction was rolled back");

        let res = block_on(on_db_typed(&worker, |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))?)
        }));
        assert_eq!(res.unwrap(), 0);
    }

    #[test]
    fn on_db_typed_keeps_the_error_type() {
        let worker = DbWorker::spawn(crate::open_in_memory().unwrap());
        let res = block_on(on_db_typed(&worker, |_c| -> Result<i64, DbError> {
            Err(DbError::UnknownRule(7))
        }));
        assert!(matches!(res, Err(DbError::UnknownRule(7))));
    }
}
