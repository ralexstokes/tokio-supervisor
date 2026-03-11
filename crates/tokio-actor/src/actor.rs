use std::{future::Future, pin::Pin, sync::Arc};

use crate::context::ActorContext;

/// A type-erased, thread-safe error type used by actor functions.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// The result type returned by every actor function.
pub type ActorResult = Result<(), BoxError>;

pub(crate) type ActorFuture = Pin<Box<dyn Future<Output = ActorResult> + Send + 'static>>;

/// Specification for a native Rust actor within a graph.
///
/// An `ActorSpec` stores an actor id plus an async factory function that is
/// invoked each time the enclosing graph is run. The factory receives a fresh
/// [`ActorContext`] containing the actor's mailbox, linked peers, and shared
/// shutdown token.
#[derive(Clone)]
pub struct ActorSpec {
    pub(crate) inner: Arc<ActorSpecInner>,
}

#[derive(Clone)]
pub(crate) struct ActorSpecInner {
    pub(crate) id: String,
    pub(crate) factory: Arc<dyn ActorFactory>,
}

pub(crate) trait ActorFactory: Send + Sync + 'static {
    fn make(&self, ctx: ActorContext) -> ActorFuture;
}

struct ClosureFactory<F> {
    f: F,
}

impl<F, Fut> ActorFactory for ClosureFactory<F>
where
    F: Fn(ActorContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ActorResult> + Send + 'static,
{
    fn make(&self, ctx: ActorContext) -> ActorFuture {
        Box::pin((self.f)(ctx))
    }
}

fn make_actor_factory<F, Fut>(f: F) -> Arc<dyn ActorFactory>
where
    F: Fn(ActorContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ActorResult> + Send + 'static,
{
    Arc::new(ClosureFactory { f })
}

impl ActorSpec {
    /// Creates a new native Rust actor specification.
    ///
    /// `id` must be unique within the graph built by
    /// [`GraphBuilder`](crate::GraphBuilder).
    pub fn native<F, Fut>(id: impl Into<String>, f: F) -> Self
    where
        F: Fn(ActorContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ActorResult> + Send + 'static,
    {
        Self {
            inner: Arc::new(ActorSpecInner {
                id: id.into(),
                factory: make_actor_factory(f),
            }),
        }
    }

    /// Returns the actor's unique identifier.
    pub fn id(&self) -> &str {
        &self.inner.id
    }
}
