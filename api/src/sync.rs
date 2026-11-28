//! [`Mutex`]: shared, mutable state in a `static`, without `static mut`.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A value only one holder at a time can reach.
///
/// It never waits: [`try_lock`](Self::try_lock) takes the lock or says it
/// is taken. A wait here could never end, since on bare metal the holder
/// may be the very code an interrupt handler interrupted, or a main loop
/// that keeps what it locked for good.
///
/// Between cores it excludes only as far as the board makes
/// `compare_exchange` atomic between them: on RP2350 the `pico2` crate's
/// startup does, on each core.
///
/// Storage that one thread of execution owns for the whole boot is locked
/// once, and the guard kept. It is then in `.bss` or `.data`, where the
/// linker counts it, rather than in a stack frame, where nothing does:
///
/// ```ignore
/// static PIPE: Mutex<Pipe<Msg, 8>> = Mutex::new(Pipe::new(MID));
///
/// fn main() -> ! {
///     let pipe = PIPE.try_lock().unwrap();
///     bus.subscribe(&*pipe).unwrap();
///     loop { /* run the scheduler */ }
/// }
/// ```
///
/// What it holds need not be [`Sync`]: a [`Pipe`](crate::sb::Pipe) is not,
/// and only the holder reaches it. It must be [`Send`], as any thread of
/// execution may be the holder. A guard is `Sync` only if the value is:
///
/// ```compile_fail,E0277
/// fn sync<T: Sync>(_: &T) {}
/// let pipe = api::sync::Mutex::new(api::sb::Pipe::<u32, 2>::new(1));
/// sync(&pipe.try_lock().unwrap());
/// ```
///
/// ```compile_fail,E0277
/// static NOT_SEND: api::sync::Mutex<*mut u32> = api::sync::Mutex::new(core::ptr::null_mut());
/// ```
///
/// What must be `&'static mut`, such as a DMA's ring or a queue to split
/// between two cores, is [`leak`](MutexGuard::leak)ed from its guard.
pub struct Mutex<T> {
    /// `true` while a guard exists.
    locked: AtomicBool,
    /// The value, reached only through the guard.
    value: UnsafeCell<T>,
}

// SAFETY: `locked` lets one guard exist at a time, and only a guard reaches
// the value; whichever thread holds it has the value to itself, so the
// value must be `Send`.
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// An unlocked mutex holding `value`, for a `static`.
    pub const fn new(value: T) -> Self {
        Self { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    /// The value, until the guard is dropped; `None` if it is already
    /// locked.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        // Acquire pairs with the release of the holder before.
        self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).ok()?;
        Some(MutexGuard { mutex: self, _exclusive: PhantomData })
    }
}

/// The right to a [`Mutex`]'s value. Dropping it unlocks.
pub struct MutexGuard<'a, T> {
    /// What it has locked.
    mutex: &'a Mutex<T>,
    /// It stands for a `&mut T`, so it is `Send` and `Sync` as one is.
    _exclusive: PhantomData<&'a mut T>,
}

impl<'a, T> MutexGuard<'a, T> {
    /// Keep the lock for good, for the value borrowed as long as the mutex
    /// is: `&'static mut` from a `static`. Every later
    /// [`try_lock`](Mutex::try_lock) returns `None`.
    ///
    /// It lives no longer than the mutex:
    ///
    /// ```compile_fail,E0597
    /// let mutex = api::sync::Mutex::new(1);
    /// let value: &'static mut i32 = mutex.try_lock().unwrap().leak();
    /// ```
    pub fn leak(self) -> &'a mut T {
        let mutex = self.mutex;
        // Never unlocked, so there is never another guard.
        core::mem::forget(self);
        // SAFETY: the one guard is gone without unlocking, so this is the
        // only route to the value for as long as the mutex lives.
        unsafe { &mut *mutex.value.get() }
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: this is the only guard, borrowed shared.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: this is the only guard, borrowed exclusively.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        // Publish what the holder wrote with the unlock.
        self.mutex.locked.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::sb::Pipe;

    #[test]
    fn one_guard_at_a_time() {
        let mutex = Mutex::new(1);
        let mut guard = mutex.try_lock().unwrap();
        assert!(mutex.try_lock().is_none());
        *guard = 2;
        drop(guard);
        assert_eq!(*mutex.try_lock().unwrap(), 2);
    }

    #[test]
    fn a_static_holds_what_is_not_sync() {
        static PIPE: Mutex<Pipe<u32, 2>> = Mutex::new(Pipe::new(1));
        let pipe = PIPE.try_lock().unwrap();
        pipe.push(7).unwrap();
        assert_eq!(pipe.pop(), Some(7));
    }

    #[test]
    fn a_leaked_guard_never_unlocks() {
        static VALUE: Mutex<[u32; 2]> = Mutex::new([0; 2]);
        let [a, b]: &'static mut [u32; 2] = VALUE.try_lock().unwrap().leak();
        (*a, *b) = (1, 2);
        assert!(VALUE.try_lock().is_none());
        assert_eq!((*a, *b), (1, 2));
    }

    #[test]
    fn threads_take_turns() {
        let mutex = Mutex::new(0u32);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..20_000 {
                        let mut guard = loop {
                            match mutex.try_lock() {
                                Some(guard) => break guard,
                                None => std::thread::yield_now(),
                            }
                        };
                        *guard += 1;
                    }
                });
            }
        });
        assert_eq!(*mutex.try_lock().unwrap(), 80_000);
    }
}
