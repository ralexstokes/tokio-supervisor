use std::collections::HashMap;

use tokio_actor::{Graph, IngressHandle};
use tokio_supervisor::ChildSpec;

/// Convenience wrapper for supervising a whole actor graph as a single child.
#[derive(Clone, Debug)]
pub struct SupervisedGraph {
    id: String,
    graph: Graph,
}

impl SupervisedGraph {
    /// Creates a new whole-graph child wrapper.
    pub fn new(id: impl Into<String>, graph: Graph) -> Self {
        Self {
            id: id.into(),
            graph,
        }
    }

    /// Returns a stable handle to a named ingress, if it exists.
    pub fn ingress(&self, name: &str) -> Option<IngressHandle> {
        self.graph.ingress(name)
    }

    /// Returns stable handles for every named ingress.
    pub fn ingresses(&self) -> HashMap<String, IngressHandle> {
        self.graph.ingresses()
    }

    /// Adapts the wrapped graph into a [`ChildSpec`].
    pub fn into_child_spec(self) -> ChildSpec {
        let id = self.id;
        let graph = self.graph;

        ChildSpec::new(id, move |ctx| {
            let graph = graph.clone();
            async move {
                graph
                    .run_until(ctx.token.cancelled())
                    .await
                    .map_err(Into::into)
            }
        })
    }
}
