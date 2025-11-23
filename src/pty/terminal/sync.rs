use parking_lot::{Mutex, MutexGuard};

/// A fair mutex that provides fairness guarantees for lock acquisition.
///
/// This prevents starvation when multiple threads compete for the lock.
/// The event loop uses lease() to reserve its turn before attempting to lock.
///
/// Based on Alacritty's FairMutex implementation from alacritty_terminal/src/sync.rs
pub struct FairMutex<T> {
    data: Mutex<T>,
    next: Mutex<()>,
}

impl<T> FairMutex<T> {
    /// Create a new FairMutex wrapping the given data.
    pub fn new(data: T) -> Self {
        Self {
            data: Mutex::new(data),
            next: Mutex::new(()),
        }
    }

    /// Reserve the next lock (fairness guarantee).
    ///
    /// The event loop calls this before try_lock_unfair() to prevent starvation.
    /// This ensures that even if multiple threads are competing for the lock,
    /// the event loop will eventually get its turn.
    pub fn lease(&self) -> MutexGuard<'_, ()> {
        self.next.lock()
    }

    /// Try to acquire the lock without blocking (unfair).
    ///
    /// Returns None if the lock is currently held.
    /// This is used by the event loop to avoid blocking when the terminal is locked.
    pub fn try_lock_unfair(&self) -> Option<MutexGuard<'_, T>> {
        self.data.try_lock()
    }

    /// Acquire the lock, blocking if necessary (unfair).
    ///
    /// Only use when you must acquire the lock (e.g., buffer full).
    /// This bypasses the fairness queue for performance.
    pub fn lock_unfair(&self) -> MutexGuard<'_, T> {
        self.data.lock()
    }

    /// Acquire the lock with fairness guarantee.
    ///
    /// This respects the lease queue for fair lock acquisition.
    /// Use this for external API calls to prevent starvation.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        // Acquire next lock first for fairness
        drop(self.next.lock());
        self.data.lock()
    }
}

