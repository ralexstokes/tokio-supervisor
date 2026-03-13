//! OTP-style composition helpers for `tokio-actor` and `tokio-supervisor`.
//!
//! `tokio-otp` keeps the actor and supervisor crates independent while
//! removing the boilerplate of composing them together.

mod error;
mod supervised_actors;
mod supervised_graph;

pub mod prelude {
    pub use tokio_actor::{
        Actor, ActorContext, ActorRef, ActorRunError, ActorSet, ActorSpec, Envelope, Graph,
        GraphBuilder, IngressHandle, RunnableActor,
    };
    pub use tokio_supervisor::{
        ChildContext, ChildSpec, Restart, RestartIntensity, ShutdownMode, ShutdownPolicy, Strategy,
        Supervisor, SupervisorBuilder, SupervisorEvent, SupervisorHandle, SupervisorSnapshot,
    };

    pub use crate::{SupervisedActors, SupervisedGraph};
}

pub use error::BuildError;
pub use supervised_actors::SupervisedActors;
pub use supervised_graph::SupervisedGraph;
