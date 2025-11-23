use kodegen_tools_terminal::pty::terminal::sync::FairMutex;
use std::sync::Arc;
use std::thread;

#[test]
fn test_basic_lock() {
    let mutex = FairMutex::new(42);
    let guard = mutex.lock();
    assert_eq!(*guard, 42);
}

#[test]
fn test_try_lock() {
    let mutex = Arc::new(FairMutex::new(0));
    let _guard = mutex.lock();

    // Should fail while locked
    assert!(mutex.try_lock_unfair().is_none());
}

#[test]
fn test_concurrent_access() {
    let mutex = Arc::new(FairMutex::new(0));
    let mut handles = vec![];

    for _ in 0..10 {
        let mutex_clone = mutex.clone();
        handles.push(thread::spawn(move || {
            let mut guard = mutex_clone.lock();
            *guard += 1;
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked during test");
    }

    assert_eq!(*mutex.lock(), 10);
}
