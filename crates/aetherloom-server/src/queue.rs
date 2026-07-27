//! Bounded channels used between networking, simulation, and replication workers.

use std::error::Error;
use std::fmt;
use std::sync::mpsc::{
    sync_channel, Receiver, SyncSender, TryRecvError, TrySendError,
};

/// Error returned when a bounded channel cannot be constructed safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueConfigError {
    ZeroCapacity,
}

impl fmt::Display for QueueConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded channel capacity must be greater than zero")
    }
}

impl Error for QueueConfigError {}

/// A non-blocking bounded-channel send failure.
#[derive(Debug, Eq, PartialEq)]
pub enum QueueSendError<T> {
    Full(T),
    Disconnected(T),
}

/// A non-blocking bounded-channel receive failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueReceiveError {
    Empty,
    Disconnected,
}

/// Cloneable producer for a bounded channel.
#[derive(Debug)]
pub struct BoundedSender<T> {
    inner: SyncSender<T>,
    capacity: usize,
}

impl<T> Clone for BoundedSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            capacity: self.capacity,
        }
    }
}

impl<T> BoundedSender<T> {
    pub fn try_send(&self, message: T) -> Result<(), QueueSendError<T>> {
        match self.inner.try_send(message) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(message)) => Err(QueueSendError::Full(message)),
            Err(TrySendError::Disconnected(message)) => {
                Err(QueueSendError::Disconnected(message))
            }
        }
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Single consumer for a bounded channel.
#[derive(Debug)]
pub struct BoundedReceiver<T> {
    inner: Receiver<T>,
    capacity: usize,
}

impl<T> BoundedReceiver<T> {
    pub fn try_receive(&self) -> Result<T, QueueReceiveError> {
        match self.inner.try_recv() {
            Ok(message) => Ok(message),
            Err(TryRecvError::Empty) => Err(QueueReceiveError::Empty),
            Err(TryRecvError::Disconnected) => Err(QueueReceiveError::Disconnected),
        }
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Creates a bounded, non-blocking channel.
///
/// Runtime code should use `try_send` rather than block the simulation thread.
pub fn bounded_channel<T>(
    capacity: usize,
) -> Result<(BoundedSender<T>, BoundedReceiver<T>), QueueConfigError> {
    if capacity == 0 {
        return Err(QueueConfigError::ZeroCapacity);
    }

    let (sender, receiver) = sync_channel(capacity);
    Ok((
        BoundedSender {
            inner: sender,
            capacity,
        },
        BoundedReceiver {
            inner: receiver,
            capacity,
        },
    ))
}

/// Simulation-thread endpoints for ingress and snapshot/event egress.
#[derive(Debug)]
pub struct SimulationIo<Ingress, Egress> {
    pub ingress: BoundedReceiver<Ingress>,
    pub egress: BoundedSender<Egress>,
}

/// Network-worker endpoints paired with [`SimulationIo`].
#[derive(Debug)]
pub struct NetworkIo<Ingress, Egress> {
    pub ingress: BoundedSender<Ingress>,
    pub egress: BoundedReceiver<Egress>,
}

/// Creates the two bounded channel pairs used by a dedicated match runtime.
pub fn runtime_channels<Ingress, Egress>(
    ingress_capacity: usize,
    egress_capacity: usize,
) -> Result<
    (
        SimulationIo<Ingress, Egress>,
        NetworkIo<Ingress, Egress>,
    ),
    QueueConfigError,
> {
    let (ingress_sender, ingress_receiver) = bounded_channel(ingress_capacity)?;
    let (egress_sender, egress_receiver) = bounded_channel(egress_capacity)?;

    Ok((
        SimulationIo {
            ingress: ingress_receiver,
            egress: egress_sender,
        },
        NetworkIo {
            ingress: ingress_sender,
            egress: egress_receiver,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_channel_reports_backpressure_without_blocking() {
        let (sender, receiver) = bounded_channel(1).expect("valid capacity");

        sender.try_send(10).expect("first item fits");
        assert_eq!(sender.try_send(20), Err(QueueSendError::Full(20)));
        assert_eq!(receiver.try_receive(), Ok(10));
        sender.try_send(20).expect("capacity is available again");
        assert_eq!(receiver.try_receive(), Ok(20));
        assert_eq!(receiver.try_receive(), Err(QueueReceiveError::Empty));
    }

    #[test]
    fn zero_capacity_is_rejected() {
        assert_eq!(
            bounded_channel::<u8>(0).unwrap_err(),
            QueueConfigError::ZeroCapacity
        );
    }

    #[test]
    fn runtime_channels_keep_both_directions_bounded() {
        let (simulation, network) =
            runtime_channels::<u8, &'static str>(2, 1).expect("valid capacities");

        network.ingress.try_send(7).expect("network ingress");
        assert_eq!(simulation.ingress.try_receive(), Ok(7));

        simulation.egress.try_send("snapshot").expect("simulation egress");
        assert_eq!(network.egress.try_receive(), Ok("snapshot"));
        assert_eq!(simulation.ingress.capacity(), 2);
        assert_eq!(network.egress.capacity(), 1);
    }
}
