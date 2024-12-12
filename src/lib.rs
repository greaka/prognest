//! # Prognest
//! A progress library.
//!
//! This is a very simple library to map subtask progress to an absolute range
//! externally. An async listener can subscribe to updates to the absolute
//! progress. Tasks can spawn and nest infinitely many subtasks and also
//! update the progress.
//!
//! ### Example
//!
//! ```rust
//! use prognest::Progress;
//!
//! let prog = Progress::new(10000);
//! let mut rx = prog.subscribe();
//!
//! let mut subtask = prog.allocate(8000);
//! subtask.set_internal(10000);
//!
//! // 5000 progress / 10000 internal * 8000 alloc = 4000
//! subtask.advance(5000);
//! assert_eq!(*rx.borrow_and_update(), 4000);
//! ```
//!
//! ### Design Considerations
//! This library was kept as simple as possible. There is no check against
//! overshooting allocated or global progress. There is also no way to know how
//! much progress a single subtask contributed so far or how far along it
//! already progressed.
//!
//! The assumption is that a task will set up subtasks and/or
//! [set the internal max value](Progress::set_internal) before starting to
//! progress.

use std::ops::{Add, AddAssign, Div, Mul, Rem};

use tokio::sync::{
    watch,
    watch::{Receiver, Sender},
};

/// A progress struct that maps internal progress to external absolute progress.
/// Maps internal to external progress.
/// Internal default value assumes noop when `add`ed to itself.
pub struct Progress<T, I = T> {
    sender: Sender<T>,
    // global max allocation
    // should be less or equal to parent allocation
    allocation: T,
    // internal max allocation
    internal: I,
    // internal unaccounted for progress
    // invariant: internal + unaccounted::default() == internal
    unaccounted: I,
}

impl<T: Default> Progress<T> {
    /// Creates a new Progress instance. See also [`Progress::with_internal`]
    pub fn new(allocation: T) -> Self {
        let (tx, _) = watch::channel(Default::default());
        Self::with_internal(tx, allocation, Default::default())
    }
}

impl<T, I> Progress<T, I> {
    /// Returns a tokio receiver to observe changes to the progress channel
    pub fn subscribe(&self) -> Receiver<T> {
        self.sender.subscribe()
    }

    /// Absolute allocation assigned
    pub fn allocation(&self) -> &T {
        &self.allocation
    }

    pub fn set_internal(&mut self, internal: I) {
        self.internal = internal;
    }
}

impl<T, I: Default> Progress<T, I> {
    /// Creates a new Progress instance and initializes the internal max value
    pub fn with_internal(sender: Sender<T>, allocation: T, internal: I) -> Progress<T, I> {
        Progress {
            sender,
            allocation,
            internal,
            unaccounted: Default::default(),
        }
    }

    /// Returns a new Progress instance with the same global channel and the
    /// given allocation. The allocation should generally be smaller or
    /// equal to the allocation of the parent task.
    pub fn allocate(&self, allocation: T) -> Progress<T, I> {
        Progress::with_internal(self.sender.clone(), allocation, Default::default())
    }
}

impl<T: Clone, I> Progress<T, I> {
    /// Convenience function to allocate a certain fraction of the allocation to
    /// a subtask.
    /// ```
    /// # use prognest::Progress;
    /// # fn subtask(prog: Progress<u32, u32>, tasks: u32) {
    /// for i in 0..tasks {
    ///     let p = prog.allocate_fraction(tasks);
    /// #       update(p, i as _);
    /// }
    /// # }
    /// # fn update(mut _prog: Progress<u32, u64>, _i: u64) {}
    /// ```
    pub fn allocate_fraction<F, K: Default>(&self, fraction: F) -> Progress<T, K>
    where
        T: Div<F, Output = T>,
    {
        let allocation = self.allocation.clone().div(fraction);
        Progress::with_internal(self.sender.clone(), allocation, Default::default())
    }
}

impl<T, I> Progress<T, I> {
    /// Advance the absolute progress
    ///
    /// skips internal progress calculation, use with caution
    pub fn advance_raw<P>(&self, progress: P)
    where
        T: AddAssign<P>,
    {
        self.sender.send_modify(|s| s.add_assign(progress));
    }

    /// see [`advance`](Progress::advance)
    pub fn advance_mul<P>(&mut self, progress: P)
    where
        T: AddAssign<<<<P as Add<I>>::Output as Mul<T>>::Output as Div<I>>::Output> + Clone,
        I: Clone,
        P: Add<I>,
        <P as Add<I>>::Output: Mul<T>,
        <<P as Add<I>>::Output as Mul<T>>::Output: Div<I> + Rem<I, Output = I> + Clone,
    {
        let intermediate = progress
            .add(self.unaccounted.clone())
            .mul(self.allocation.clone());
        let prog = intermediate.clone().div(self.internal.clone());
        let unaccounted = intermediate.rem(self.internal.clone());
        self.unaccounted = unaccounted;
        self.advance_raw(prog);
    }

    /// Advances absolute progress according to internal progress made. Takes
    /// units of internal progress and emits absolute progress.
    ///
    /// if advance doesn't compile, try [`advance_mul`](Progress::advance_mul)
    pub fn advance<P>(&mut self, progress: P)
    where
        T: AddAssign<T> + TryInto<I> + Clone,
        I: Clone,
        P: Add<I>,
        <P as Add<I>>::Output: Mul<I>,
        <<P as Add<I>>::Output as Mul<I>>::Output: Div<I> + Rem<I, Output = I> + Clone,
        <<<P as Add<I>>::Output as Mul<I>>::Output as Div<I>>::Output: TryInto<T>,
    {
        let intermediate = progress
            .add(self.unaccounted.clone())
            .mul(self.allocation.clone().try_into().ok().unwrap());
        let prog = intermediate
            .clone()
            .div(self.internal.clone())
            .try_into()
            .ok()
            .unwrap();
        let unaccounted = intermediate.rem(self.internal.clone());
        self.unaccounted = unaccounted;
        self.advance_raw(prog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let prog = Progress::new(10000);
        let mut rx = prog.subscribe();
        assert_eq!(*rx.borrow_and_update(), 0);
        prog.advance_raw(100);
        prog.advance_raw(200);
        assert_eq!(*rx.borrow_and_update(), 300);
    }

    #[test]
    fn simple_subtask() {
        let prog = Progress::new(10000);
        let mut rx = prog.subscribe();
        assert_eq!(*rx.borrow_and_update(), 0);

        let mut subtask = prog.allocate(5000);
        subtask.set_internal(10000);

        // not recommended
        // fully exhausts allocation
        subtask.advance_raw(5000);
        assert_eq!(*rx.borrow_and_update(), 5000);

        // 5000 alloc / 10000 internal * 5000 progress = 2500
        subtask.advance(5000);
        assert_eq!(*rx.borrow_and_update(), 7500);

        // allocates subtask with 2500 global units allowance
        let mut subsubtask = subtask.allocate_fraction(2);
        subsubtask.set_internal(100);
        subsubtask.advance(100);
        // not recommended
        subsubtask.advance(100);

        // we are well past the root global alloc
        assert_eq!(*rx.borrow_and_update(), 12500);
    }

    fn subtask(prog: Progress<u32, u32>, tasks: u32) {
        for i in 0..tasks {
            let p = prog.allocate_fraction(tasks);
            update(p, i as _);
        }
    }

    fn update(mut prog: Progress<u32, u64>, i: u64) {
        prog.set_internal(400000);
        prog.advance(50000 * i);
    }

    #[test]
    fn subtasks() {
        let prog = Progress::new(10000);
        let mut rx = prog.subscribe();
        assert_eq!(*rx.borrow_and_update(), 0);
        const FRACTION: u32 = 8;
        subtask(prog, FRACTION);
        let fraction = 10000 / FRACTION / FRACTION;
        let progress = FRACTION * (FRACTION - 1) / 2;
        let control = progress * fraction;
        assert!((*rx.borrow_and_update() as i32 - control as i32).abs() <= FRACTION as i32);
    }

    #[test]
    fn thread() {
        let prog = Progress::new(10000);
        let mut rx = prog.subscribe();
        assert_eq!(*rx.borrow_and_update(), 0);
        let thread = std::thread::spawn(move || {
            subtask(prog, 2);
        });
        thread.join().unwrap();
        assert_eq!(*rx.borrow_and_update(), 625);
    }
}
