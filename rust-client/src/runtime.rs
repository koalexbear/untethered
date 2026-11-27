//! The desktop's async runtime, and how much memory its threads keep.
//!
//! Tokio's default runtime starts a worker per CPU thread and allows 512
//! blocking threads, and the SDK's SQLite stores and image decoding run on
//! those. On a 32-thread CPU that came to about a hundred threads, and glibc
//! gives each thread that allocates its own malloc arena: memory freed in one
//! often stayed held, hundreds of megabytes of it. A messenger needs a few
//! workers, a bounded blocking pool and a couple of arenas.

/// Async workers: the UI's tasks and the Matrix engine.
const WORKERS: usize = 4;
/// Threads for blocking work: the SDK's SQLite connections (one per
/// physical core at most), image decoding and file I/O.
const BLOCKING: usize = 24;
/// glibc malloc arenas for the whole process.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const ARENAS: i32 = 2;

pub struct Runtime(tokio::runtime::Runtime);

impl iced::Executor for Runtime {
    fn new() -> Result<Self, std::io::Error> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .max_blocking_threads(BLOCKING)
            .enable_all()
            .build()
            .map(Self)
    }

    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        drop(self.0.spawn(future));
    }

    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.0.block_on(future)
    }

    fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _runtime = self.0.enter();
        f()
    }
}

/// Before any thread starts: cap glibc's malloc arenas, as
/// `MALLOC_ARENA_MAX` would.
pub fn limit_arenas() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: mallopt only sets an allocator parameter; it is called before
    // the process starts other threads.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, ARENAS);
    }
}
