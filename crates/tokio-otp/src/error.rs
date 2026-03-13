use thiserror::Error;
use tokio_actor::GraphError;

/// Errors returned while adapting actor graphs into supervisor children.
#[derive(Debug, Error)]
pub enum BuildError {
    /// The graph could not be decomposed into an actor set.
    #[error(transparent)]
    Graph(#[from] GraphError),
    /// The supplied supervisor configuration was invalid.
    #[error(transparent)]
    Supervisor(#[from] tokio_supervisor::BuildError),
    /// An override referenced an actor id that does not exist in the graph.
    #[error("unknown actor `{actor_id}`")]
    UnknownActor { actor_id: String },
}
