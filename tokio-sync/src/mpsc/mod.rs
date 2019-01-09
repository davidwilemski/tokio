//! A multi-producer, single-consumer queue for sending values across
//! asynchronous tasks.
//!
//! Similar to `std`, channel creation provides [`Receiver`] and [`Sender`]
//! handles. [`Receiver`] implements [`Stream`] and allows a task to
//! read values out of the channel. If there is no message to read, the current
//! task will be notified when a new value is sent.  [`Sender`] implements the
//! `Sink` trait and allows sending messages into the channel. If the channel is
//! at capacity, the send is rejected and the task will be notified when
//! additional capacity is available. In other words, the channel provides
//! backpressure.
//!
//! Unbounded channels are also available using the `unbounded_channel`
//! constructor.
//!
//! # Disconnection
//!
//! When all [`Sender`] handles have been dropped, it is no longer
//! possible to send values into the channel. This is considered the termination
//! event of the stream. As such, [`Receiver::poll`] returns `Ok(Ready(None))`.
//!
//! If the [`Receiver`] handle is dropped, then messages can no longer
//! be read out of the channel. In this case, all further attempts to send will
//! result in an error.
//!
//! # Clean Shutdown
//!
//! When the [`Receiver`] is dropped, it is possible for unprocessed messages to
//! remain in the channel. Instead, it is usually desirable to perform a "clean"
//! shutdown. To do this, the receiver first calls `close`, which will prevent
//! any further messages to be sent into the channel. Then, the receiver
//! consumes the channel to completion, at which point the receiver can be
//! dropped.
//!
//! [`Sender`]: struct.Sender.html
//! [`Receiver`]: struct.Receiver.html
//! [`Stream`]: ../../futures_core/stream/trait.Stream.html
//! [`Receiver::poll_next`]: ../../futures_core/stream/trait.Stream.html#tymethod.poll_next

mod block;
mod bounded;
mod chan;
mod list;
mod unbounded;

pub use self::bounded::{
    channel,
    Receiver,
    Sender
};

pub use self::unbounded::{
    unbounded_channel,
    UnboundedReceiver,
    UnboundedSender,
};

pub mod error {
    pub use super::bounded::{
        SendError,
        TrySendError,
        RecvError,
    };

    pub use super::unbounded::{
        UnboundedSendError,
        UnboundedTrySendError,
        UnboundedRecvError,
    };
}

/// The number of values a block can contain.
///
/// This value must be a power of 2. It also must be smaller than the number of
/// bits in `usize`.
#[cfg(target_pointer_width = "64")]
const BLOCK_CAP: usize = 32;

#[cfg(not(target_pointer_width = "64"))]
const BLOCK_CAP: usize = 16;
