use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct ChildContext {
    pub id: String,
    pub generation: u64,
    pub token: CancellationToken,
    pub supervisor_token: SupervisorToken,
}

/// Read-only view of the supervisor's cancellation token.
///
/// Children can observe supervisor-level cancellation but cannot trigger it.
#[derive(Clone, Debug)]
pub struct SupervisorToken(CancellationToken);

impl SupervisorToken {
    pub(crate) fn new(token: CancellationToken) -> Self {
        Self(token)
    }

    /// Returns a future that completes when the supervisor is cancelled.
    pub async fn cancelled(&self) {
        self.0.cancelled().await;
    }

    /// Returns `true` if the supervisor has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}
