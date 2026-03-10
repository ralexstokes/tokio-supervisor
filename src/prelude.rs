//! Common imports and extension traits for `tokio-supervisor` consumers.
//!
//! ```
//! use tokio_supervisor::prelude::*;
//! ```

use tokio::sync::{broadcast, watch};

#[allow(async_fn_in_trait)]
pub trait SupervisorEventReceiverExt {
    async fn wait_for_event<P>(
        &mut self,
        predicate: P,
    ) -> Result<crate::SupervisorEvent, broadcast::error::RecvError>
    where
        P: FnMut(&crate::SupervisorEvent) -> bool;
}

impl SupervisorEventReceiverExt for broadcast::Receiver<crate::SupervisorEvent> {
    async fn wait_for_event<P>(
        &mut self,
        mut predicate: P,
    ) -> Result<crate::SupervisorEvent, broadcast::error::RecvError>
    where
        P: FnMut(&crate::SupervisorEvent) -> bool,
    {
        loop {
            let event = self.recv().await?;
            if predicate(&event) {
                return Ok(event);
            }
        }
    }
}

#[allow(async_fn_in_trait)]
pub trait SupervisorSnapshotReceiverExt {
    async fn wait_for_snapshot<P>(
        &mut self,
        predicate: P,
    ) -> Result<crate::SupervisorSnapshot, watch::error::RecvError>
    where
        P: FnMut(&crate::SupervisorSnapshot) -> bool;
}

impl SupervisorSnapshotReceiverExt for watch::Receiver<crate::SupervisorSnapshot> {
    async fn wait_for_snapshot<P>(
        &mut self,
        mut predicate: P,
    ) -> Result<crate::SupervisorSnapshot, watch::error::RecvError>
    where
        P: FnMut(&crate::SupervisorSnapshot) -> bool,
    {
        let current = self.borrow().clone();
        if predicate(&current) {
            return Ok(current);
        }

        loop {
            self.changed().await?;
            let snapshot = self.borrow().clone();
            if predicate(&snapshot) {
                return Ok(snapshot);
            }
        }
    }
}

pub use crate::{
    BackoffPolicy, BoxError, BuildError, ChildContext, ChildMembershipView, ChildResult,
    ChildSnapshot, ChildSpec, ChildStateView, ControlError, EventPathSegment, ExitStatusView,
    Restart, RestartIntensity, ShutdownMode, ShutdownPolicy, Strategy, Supervisor,
    SupervisorBuilder, SupervisorError, SupervisorEvent, SupervisorExit, SupervisorHandle,
    SupervisorSnapshot, SupervisorStateView, SupervisorToken,
};
