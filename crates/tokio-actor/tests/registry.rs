use tokio_actor::{ActorContext, ActorRegistry, ActorSpec, GraphBuilder};

#[test]
fn registry_tracks_registered_actor_refs() {
    let graph = GraphBuilder::new()
        .actor(ActorSpec::from_actor(
            "worker",
            |_ctx: ActorContext| async move { Ok(()) },
        ))
        .build()
        .expect("valid graph");

    let actor_set = graph.into_actor_set().expect("graph decomposes");
    let worker = actor_set.actor("worker").expect("worker exists");
    let registry = ActorRegistry::new();

    worker
        .register_with(&registry)
        .expect("worker registers successfully");

    assert!(registry.contains("worker"));
    assert_eq!(registry.actor_ids(), vec!["worker".to_owned()]);
    assert_eq!(
        registry.actor_ref("worker").expect("actor ref exists").id(),
        "worker"
    );

    registry
        .deregister("worker")
        .expect("worker deregisters successfully");
    assert!(!registry.contains("worker"));
    assert!(registry.actor_ref("worker").is_none());
}
