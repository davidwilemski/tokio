use super::list;
use futures::Poll;
use futures::task::AtomicTask;

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{AcqRel, Relaxed};

/// Channel sender
pub(crate) struct Tx<T, S: Semaphore> {
    inner: Arc<Chan<T, S>>,
    permit: S::Permit,
}

/// Channel receiver
pub(crate) struct Rx<T, S> {
    inner: Arc<Chan<T, S>>,
}

pub trait Semaphore: Sync {
    type Permit;

    fn new_permit() -> Self::Permit;

    fn drop_permit(&self, permit: &mut Self::Permit);

    fn is_idle(&self) -> bool;

    fn add_permits(&self, num: usize);

    fn poll_acquire(&self, permit: &mut Self::Permit) -> Poll<(), ()>;

    fn try_acquire(&self, permit: &mut Self::Permit) -> Result<(), ()>;

    fn forget(&self, permit: &mut Self::Permit);

    fn close(&self);
}

struct Chan<T, S> {
    /// Handle to the push half of the lock-free list.
    tx: list::Tx<T>,

    /// Coordinates access to channel's capacity.
    semaphore: S,

    /// Receiver task. Notified when a value is pushed into the channel.
    rx_task: AtomicTask,

    /// Tracks the number of outstanding sender handles.
    ///
    /// When this drops to zero, the send half of the channel is closed.
    tx_count: AtomicUsize,

    /// Only accessed by `Rx` handle.
    rx_fields: UnsafeCell<RxFields<T>>,
}

/// Fields only accessed by `Rx` handle.
struct RxFields<T> {
    /// Channel receiver. This field is only accessed by the `Receiver` type.
    list: list::Rx<T>,

    /// `true` if `Rx::close` is called.
    rx_closed: bool,
}

unsafe impl<T: Send, S: Send> Send for Chan<T, S> {}
unsafe impl<T: Send, S: Sync> Sync for Chan<T, S> {}

pub(crate) fn channel<T, S>(semaphore: S) -> (Tx<T, S>, Rx<T, S>)
where
    S: Semaphore,
{
    let (tx, rx) = list::channel();

    let chan = Arc::new(Chan {
        tx,
        semaphore,
        rx_task: AtomicTask::new(),
        tx_count: AtomicUsize::new(1),
        rx_fields: UnsafeCell::new(RxFields {
            list: rx,
            rx_closed: false,
        }),
    });

    (Tx::new(chan.clone()), Rx::new(chan))
}

// ===== impl Tx =====

impl<T, S> Tx<T, S>
where
    S: Semaphore,
{
    fn new(chan: Arc<Chan<T, S>>) -> Tx<T, S> {
        Tx {
            inner: chan,
            permit: S::new_permit(),
        }
    }

    /// TODO: Docs
    pub fn poll_ready(&mut self) -> Poll<(), ()> {
        self.inner.semaphore.poll_acquire(&mut self.permit)
    }

    /// Send a message and notify the receiver.
    pub fn try_send(&mut self, value: T) -> Result<(), ()> {
        self.inner.semaphore.try_acquire(&mut self.permit)?;

        // Push the value
        self.inner.tx.push(value);

        // Notify the rx task
        self.inner.rx_task.notify();

        // Release the permit
        self.inner.semaphore.forget(&mut self.permit);

        Ok(())
    }
}

impl<T, S> Clone for Tx<T, S>
where
    S: Semaphore,
{
    fn clone(&self) -> Tx<T, S> {
        // Using a Relaxed ordering here is sufficient as the caller holds a
        // strong ref to `self`, preventing a concurrent decrement to zero.
        self.inner.tx_count.fetch_add(1, Relaxed);

        Tx {
            inner: self.inner.clone(),
            permit: S::new_permit(),
        }
    }
}

impl<T, S> Drop for Tx<T, S>
where
    S: Semaphore,
{
    fn drop(&mut self) {
        self.inner.semaphore.drop_permit(&mut self.permit);

        if self.inner.tx_count.fetch_sub(1, AcqRel) != 1 {
            return;
        }

        // Close the list, which sends a `Close` message
        self.inner.tx.close();

        // Notify the receiver
        self.inner.rx_task.notify();
    }
}

// ===== impl Rx =====

impl<T, S> Rx<T, S>
where
    S: Semaphore,
{
    fn new(chan: Arc<Chan<T, S>>) -> Rx<T, S> {
        Rx { inner: chan }
    }

    pub fn close(&mut self) {
        let rx_fields = unsafe { &mut *self.inner.rx_fields.get() };

        rx_fields.rx_closed = true;
        self.inner.semaphore.close();
    }

    /// Receive the next value
    pub fn recv(&mut self) -> Poll<Option<T>, ()> {
        use super::block::Read::*;
        use futures::Async::*;

        let rx_fields = unsafe { &mut *self.inner.rx_fields.get() };

        macro_rules! try_recv {
            () => {
                match rx_fields.list.pop(&self.inner.tx) {
                    Some(Value(value)) => {
                        self.inner.semaphore.add_permits(1);
                        return Ok(Ready(Some(value)));
                    }
                    Some(Closed) => {
                        // TODO: This check may not be required as it most
                        // likely can only return `true` at this point. A
                        // channel is closed when all tx handles are dropped.
                        // Dropping a tx handle releases memory, which ensures
                        // that if dropping the tx handle is visible, then all
                        // messages sent are also visible.
                        assert!(self.inner.semaphore.is_idle());
                        return Ok(Ready(None));
                    }
                    None => {} // fall through
                }
            }
        }

        try_recv!();

        self.inner.rx_task.register();

        // It is possible that a value was pushed between attempting to read and
        // registering the task, so we have to check the channel a second time
        // here.
        try_recv!();

        debug!("recv; rx_closed = {:?}; is_idle = {:?}",
               rx_fields.rx_closed, self.inner.semaphore.is_idle());

        if rx_fields.rx_closed && self.inner.semaphore.is_idle() {
            Ok(Ready(None))
        } else {
            Ok(NotReady)
        }
    }
}

// ===== impl Semaphore for (::Semaphore, capacity) =====

use semaphore::Permit;

impl Semaphore for (::semaphore::Semaphore, usize) {
    type Permit = Permit;

    fn new_permit() -> Permit {
        Permit::new()
    }

    fn drop_permit(&self, permit: &mut Permit) {
        if permit.is_acquired() {
            permit.release(&self.0);
        }
    }

    fn add_permits(&self, num: usize) {
        self.0.add_permits(num)
    }

    fn is_idle(&self) -> bool {
        self.0.available_permits() == self.1
    }

    fn poll_acquire(&self, permit: &mut Permit) -> Poll<(), ()> {
        permit.poll_acquire(&self.0)
            .map_err(|_| ())
    }

    fn try_acquire(&self, permit: &mut Permit) -> Result<(), ()> {
        permit.try_acquire(&self.0)
            .map_err(|_| ())
    }

    fn forget(&self, permit: &mut Self::Permit) {
        permit.forget()
    }

    fn close(&self) {
        self.0.close();
    }
}

// ===== impl Semaphore for AtomicUsize =====
