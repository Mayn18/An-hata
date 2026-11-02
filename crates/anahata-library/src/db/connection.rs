use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use rusqlite::{Connection, OpenFlags};

use crate::db::migrations::run_migrations;
use crate::error::{LibraryError, LibraryResult};

pub struct ReaderPool {
    pool: Arc<(Mutex<Vec<Connection>>, Condvar)>,
}

impl ReaderPool {
    pub fn new<P: AsRef<Path>>(db_path: P, size: usize) -> LibraryResult<Self> {
        let mut connections = Vec::with_capacity(size);
        for _ in 0..size {
            let conn = Connection::open_with_flags(
                db_path.as_ref(),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            conn.pragma_update(None, "busy_timeout", 5000)?;
            conn.pragma_update(None, "query_only", "ON")?;
            connections.push(conn);
        }

        Ok(Self {
            pool: Arc::new((Mutex::new(connections), Condvar::new())),
        })
    }

    pub fn with_connection<F, T>(&self, f: F) -> LibraryResult<T>
    where
        F: FnOnce(&Connection) -> LibraryResult<T>,
    {
        let (lock, cvar) = &*self.pool;
        let mut conns = lock.lock().unwrap_or_else(|e| e.into_inner());
        while conns.is_empty() {
            conns = cvar.wait(conns).unwrap_or_else(|e| e.into_inner());
        }
        let conn = conns.pop().expect("pool is non-empty");
        drop(conns);

        // The guard puts the connection back even if `f` panics, so a panicking query can
        // never shrink the pool (four lost connections used to deadlock every reader).
        let guard = PoolGuard { pool: &self.pool, conn: Some(conn) };
        f(guard.conn.as_ref().expect("guard holds a connection"))
    }
}

struct PoolGuard<'a> {
    pool: &'a (Mutex<Vec<Connection>>, Condvar),
    conn: Option<Connection>,
}

impl Drop for PoolGuard<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let (lock, cvar) = self.pool;
            lock.lock().unwrap_or_else(|e| e.into_inner()).push(conn);
            cvar.notify_one();
        }
    }
}

pub type WriteJob = Box<dyn FnOnce(&mut Connection) -> LibraryResult<()> + Send + 'static>;

pub struct WriterHandle {
    job_tx: Sender<Option<WriteJob>>,
    worker_handle: Option<JoinHandle<()>>,
}

impl WriterHandle {
    pub fn new<P: AsRef<Path>>(db_path: P) -> LibraryResult<Self> {
        let path_buf = db_path.as_ref().to_path_buf();
        let (job_tx, job_rx) = mpsc::channel::<Option<WriteJob>>();

        let worker_handle = thread::Builder::new()
            .name("db-writer".to_string())
            .spawn(move || {
                Self::writer_loop(path_buf, job_rx);
            })
            .map_err(|e| LibraryError::Internal(format!("Failed to spawn writer thread: {e}")))?;

        Ok(Self {
            job_tx,
            worker_handle: Some(worker_handle),
        })
    }

    pub fn execute<F>(&self, f: F) -> LibraryResult<()>
    where
        F: FnOnce(&mut Connection) -> LibraryResult<()> + Send + 'static,
    {
        let (res_tx, res_rx) = mpsc::channel();
        let job = Box::new(move |conn: &mut Connection| {
            let res = f(conn);
            let _ = res_tx.send(res);
            Ok(())
        });

        self.job_tx
            .send(Some(job))
            .map_err(|_| LibraryError::Internal("Database writer disconnected".to_string()))?;

        res_rx
            .recv()
            .map_err(|_| LibraryError::Internal("Database writer dropped response".to_string()))?
    }

    fn writer_loop(path: PathBuf, job_rx: Receiver<Option<WriteJob>>) {
        let mut conn = match Connection::open(&path) {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to open write connection at {}: {e}", path.display());
                return;
            }
        };

        if let Err(e) = conn.pragma_update(None, "journal_mode", "WAL") {
            log::error!("Failed to enable WAL: {e}");
        }
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        let _ = conn.pragma_update(None, "foreign_keys", "ON");
        let _ = conn.pragma_update(None, "busy_timeout", 5000);

        if let Err(e) = run_migrations(&mut conn) {
            log::error!("Failed to run migrations: {e}");
            return;
        }

        while let Ok(Some(job)) = job_rx.recv() {
            if let Err(e) = job(&mut conn) {
                log::error!("Write job failed: {e}");
            }
        }
    }
}

impl Drop for WriterHandle {
    fn drop(&mut self) {
        let _ = self.job_tx.send(None);
        if let Some(h) = self.worker_handle.take() {
            let _ = h.join();
        }
    }
}

pub struct Database {
    pub writer: WriterHandle,
    pub readers: ReaderPool,
    path: PathBuf,
}

impl Database {
    pub fn open<P: AsRef<Path>>(path: P) -> LibraryResult<Self> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let writer = WriterHandle::new(&path_buf)?;

        // Wait a tiny moment to ensure initial migration is finished by writer
        writer.execute(|_| Ok(()))?;

        let readers = ReaderPool::new(&path_buf, 4)?;

        Ok(Self {
            writer,
            readers,
            path: path_buf,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
