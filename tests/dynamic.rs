use tokio::{
    sync::{broadcast, mpsc},
    time::timeout,
};
use tokio_supervisor::{
    ChildSpec, ControlError, Restart, SupervisorBuilder, SupervisorEvent, SupervisorExit,
};

mod common;

#[tokio::test]
async fn add_child_starts_it_immediately() {
    let (dynamic_tx, mut dynamic_rx) = mpsc::unbounded_channel();

    let supervisor = SupervisorBuilder::new()
        .child(ChildSpec::new("seed", |ctx| async move {
            ctx.token.cancelled().await;
            Ok(())
        }))
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();

    handle
        .add_child(ChildSpec::new("dynamic", move |ctx| {
            let dynamic_tx = dynamic_tx.clone();
            async move {
                dynamic_tx
                    .send(ctx.generation)
                    .expect("test receiver dropped");
                ctx.token.cancelled().await;
                Ok(())
            }
        }))
        .await
        .expect("dynamic child should be accepted");

    assert_eq!(common::recv_event(&mut dynamic_rx).await, 0);

    handle.shutdown();
    let exit = handle.wait().await.expect("shutdown should succeed");
    assert!(matches!(exit, SupervisorExit::Shutdown));
}

#[tokio::test]
async fn remove_child_stops_it_without_restarting() {
    let (starts_tx, mut starts_rx) = mpsc::unbounded_channel();
    let supervisor = SupervisorBuilder::new()
        .child(
            ChildSpec::new("removable", move |ctx| {
                let starts_tx = starts_tx.clone();
                async move {
                    starts_tx
                        .send(ctx.generation)
                        .expect("test receiver dropped");
                    ctx.token.cancelled().await;
                    Err(common::test_error("do not restart on remove"))
                }
            })
            .restart(Restart::Transient),
        )
        .child(ChildSpec::new("keeper", |ctx| async move {
            ctx.token.cancelled().await;
            Ok(())
        }))
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();
    let mut events = handle.subscribe();

    assert_eq!(common::recv_event(&mut starts_rx).await, 0);

    handle
        .remove_child("removable")
        .await
        .expect("child removal should succeed");

    let mut saw_remove_requested = false;
    let mut saw_removed = false;
    while !saw_remove_requested || !saw_removed {
        match recv_supervisor_event(&mut events).await {
            SupervisorEvent::ChildRemoveRequested { id } if id == "removable" => {
                saw_remove_requested = true;
            }
            SupervisorEvent::ChildRemoved { id } if id == "removable" => {
                saw_removed = true;
            }
            _ => {}
        }
    }

    common::assert_no_event(&mut starts_rx).await;

    handle.shutdown();
    let exit = handle.wait().await.expect("shutdown should succeed");
    assert!(matches!(exit, SupervisorExit::Shutdown));
}

#[tokio::test]
async fn duplicate_add_and_unknown_remove_are_rejected() {
    let supervisor = SupervisorBuilder::new()
        .child(ChildSpec::new("seed", |ctx| async move {
            ctx.token.cancelled().await;
            Ok(())
        }))
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();

    let duplicate = handle
        .add_child(ChildSpec::new("seed", |_ctx| async move { Ok(()) }))
        .await
        .expect_err("duplicate id should be rejected");
    assert_eq!(duplicate, ControlError::DuplicateChildId("seed".to_owned()));

    let missing = handle
        .remove_child("missing")
        .await
        .expect_err("unknown child id should be rejected");
    assert_eq!(missing, ControlError::UnknownChildId("missing".to_owned()));

    handle.shutdown();
    let exit = handle.wait().await.expect("shutdown should succeed");
    assert!(matches!(exit, SupervisorExit::Shutdown));
}

#[tokio::test]
async fn removing_the_last_active_child_is_rejected() {
    let supervisor = SupervisorBuilder::new()
        .child(ChildSpec::new("only", |ctx| async move {
            ctx.token.cancelled().await;
            Ok(())
        }))
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();

    let err = handle
        .remove_child("only")
        .await
        .expect_err("last child removal should be rejected");
    assert_eq!(err, ControlError::LastChildRemovalUnsupported);

    handle.shutdown();
    let exit = handle.wait().await.expect("shutdown should succeed");
    assert!(matches!(exit, SupervisorExit::Shutdown));
}

async fn recv_supervisor_event(
    events: &mut broadcast::Receiver<SupervisorEvent>,
) -> SupervisorEvent {
    match timeout(common::EVENT_TIMEOUT, events.recv())
        .await
        .expect("timed out waiting for supervisor event")
    {
        Ok(event) => event,
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            panic!("lagged while reading supervisor events: skipped {skipped}");
        }
        Err(broadcast::error::RecvError::Closed) => {
            panic!("supervisor event stream closed unexpectedly");
        }
    }
}
