use std::{future::Future, pin::Pin, sync::Arc};

use crate::{
    context::ChildContext,
    restart::{Restart, RestartIntensity},
    shutdown::ShutdownPolicy,
};

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub type ChildResult = Result<(), BoxError>;

pub(crate) type ChildFuture = Pin<Box<dyn Future<Output = ChildResult> + Send + 'static>>;

#[derive(Clone)]
pub struct ChildSpec {
    pub(crate) inner: Arc<ChildSpecInner>,
}

#[derive(Clone)]
pub(crate) struct ChildSpecInner {
    pub(crate) id: String,
    pub(crate) restart: Restart,
    pub(crate) restart_intensity: Option<RestartIntensity>,
    pub(crate) shutdown_policy: ShutdownPolicy,
    pub(crate) factory: Arc<dyn ChildFactory>,
}

pub(crate) trait ChildFactory: Send + Sync + 'static {
    fn make(&self, ctx: ChildContext) -> ChildFuture;
}

struct ClosureFactory<F> {
    f: F,
}

impl<F, Fut> ChildFactory for ClosureFactory<F>
where
    F: Fn(ChildContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ChildResult> + Send + 'static,
{
    fn make(&self, ctx: ChildContext) -> ChildFuture {
        Box::pin((self.f)(ctx))
    }
}

fn make_child_factory<F, Fut>(f: F) -> Arc<dyn ChildFactory>
where
    F: Fn(ChildContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ChildResult> + Send + 'static,
{
    Arc::new(ClosureFactory { f })
}

impl ChildSpec {
    fn map_inner(mut self, update: impl FnOnce(&mut ChildSpecInner)) -> Self {
        let inner = Arc::make_mut(&mut self.inner);
        update(inner);
        self
    }

    pub fn new<F, Fut>(id: impl Into<String>, f: F) -> Self
    where
        F: Fn(ChildContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ChildResult> + Send + 'static,
    {
        Self {
            inner: Arc::new(ChildSpecInner {
                id: id.into(),
                restart: Restart::default(),
                restart_intensity: None,
                shutdown_policy: ShutdownPolicy::default(),
                factory: make_child_factory(f),
            }),
        }
    }

    #[must_use]
    pub fn restart(self, restart: Restart) -> Self {
        self.map_inner(|inner| inner.restart = restart)
    }

    #[must_use]
    pub fn shutdown(self, policy: ShutdownPolicy) -> Self {
        self.map_inner(|inner| inner.shutdown_policy = policy)
    }

    #[must_use]
    pub fn restart_intensity(self, intensity: RestartIntensity) -> Self {
        self.map_inner(|inner| inner.restart_intensity = Some(intensity))
    }

    pub fn id(&self) -> &str {
        &self.inner.id
    }

    pub fn restart_policy(&self) -> Restart {
        self.inner.restart
    }

    pub(crate) fn restart_intensity_override(&self) -> Option<RestartIntensity> {
        self.inner.restart_intensity
    }

    pub fn shutdown_policy(&self) -> ShutdownPolicy {
        self.inner.shutdown_policy
    }
}
