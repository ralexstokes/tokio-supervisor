use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use tokio::{
    sync::{broadcast, mpsc},
    time::{Duration, sleep, timeout},
};
use tokio_supervisor::{
    BackoffPolicy, ChildSpec, Restart, RestartIntensity, ShutdownMode, ShutdownPolicy,
    SupervisorBuilder, SupervisorEvent, SupervisorExit,
};

mod common;

#[tokio::test]
async fn external_shutdown_stops_all_children() {
    let exits = Arc::new(AtomicUsize::new(0));
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();

    let make_child = |id: &'static str, exits: Arc<AtomicUsize>| {
        let started_tx = started_tx.clone();
        ChildSpec::new(id, move |ctx| {
            let exits = exits.clone();
            let started_tx = started_tx.clone();
            async move {
                started_tx.send(()).expect("test receiver dropped");
                ctx.token.cancelled().await;
                exits.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
    };

    let supervisor = SupervisorBuilder::new()
        .child(make_child("worker-a", exits.clone()))
        .child(make_child("worker-b", exits.clone()))
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();
    common::recv_event(&mut started_rx).await;
    common::recv_event(&mut started_rx).await;
    handle.shutdown();

    let exit = handle.wait().await.expect("shutdown should succeed");
    assert!(matches!(exit, SupervisorExit::Shutdown));
    assert_eq!(exits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn shutdown_is_idempotent_across_handle_clones() {
    let supervisor = SupervisorBuilder::new()
        .child(ChildSpec::new("worker", |ctx| async move {
            ctx.token.cancelled().await;
            Ok(())
        }))
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();
    let clone = handle.clone();

    handle.shutdown();
    clone.shutdown();
    handle.shutdown();

    let first = handle.wait().await.expect("first waiter should resolve");
    let second = clone.wait().await.expect("second waiter should resolve");

    assert!(matches!(first, SupervisorExit::Shutdown));
    assert!(matches!(second, SupervisorExit::Shutdown));
}

#[tokio::test]
async fn cooperative_child_observes_cancellation_before_shutdown_finishes() {
    let saw_cancel = Arc::new(AtomicBool::new(false));

    let saw_cancel_for_child = saw_cancel.clone();
    let supervisor = SupervisorBuilder::new()
        .child(ChildSpec::new("worker", move |ctx| {
            let saw_cancel = saw_cancel_for_child.clone();
            async move {
                ctx.token.cancelled().await;
                saw_cancel.store(true, Ordering::SeqCst);
                Ok(())
            }
        }))
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();
    handle.shutdown();

    let exit = handle.wait().await.expect("shutdown should succeed");
    assert!(matches!(exit, SupervisorExit::Shutdown));
    assert!(saw_cancel.load(Ordering::SeqCst));
}

#[tokio::test]
async fn stubborn_child_is_aborted_in_cooperative_then_abort_mode() {
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let live_flag = common::LiveFlag::new();

    let saw_cancel_for_child = saw_cancel.clone();
    let live_flag_for_child = live_flag.clone();
    let supervisor = SupervisorBuilder::new()
        .child(
            ChildSpec::new("stubborn", move |_ctx| {
                let saw_cancel = saw_cancel_for_child.clone();
                let live_flag = live_flag_for_child.clone();
                async move {
                    let _guard = live_flag.guard();
                    loop {
                        sleep(Duration::from_millis(10)).await;
                        let _ = saw_cancel.load(Ordering::SeqCst);
                    }
                }
            })
            .shutdown(ShutdownPolicy {
                grace: common::SHORT_GRACE,
                mode: ShutdownMode::CooperativeThenAbort,
            }),
        )
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();
    handle.shutdown();

    let exit = handle.wait().await.expect("shutdown should succeed");
    assert!(matches!(exit, SupervisorExit::Shutdown));
    assert!(!saw_cancel.load(Ordering::SeqCst));
    assert!(
        !live_flag.is_live(),
        "task should be dropped before wait resolves"
    );
}

#[tokio::test]
async fn wait_only_resolves_after_child_lifetimes_end() {
    let live_flag = common::LiveFlag::new();
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();

    let live_flag_for_child = live_flag.clone();
    let supervisor = SupervisorBuilder::new()
        .child(
            ChildSpec::new("stubborn", move |_ctx| {
                let started_tx = started_tx.clone();
                let live_flag = live_flag_for_child.clone();
                async move {
                    let _guard = live_flag.guard();
                    started_tx.send(()).expect("test receiver dropped");
                    loop {
                        sleep(Duration::from_millis(10)).await;
                    }
                }
            })
            .shutdown(ShutdownPolicy {
                grace: common::SHORT_GRACE,
                mode: ShutdownMode::Abort,
            }),
        )
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();
    common::recv_event(&mut started_rx).await;
    assert!(live_flag.is_live());

    handle.shutdown();
    let exit = handle.wait().await.expect("shutdown should succeed");

    assert!(matches!(exit, SupervisorExit::Shutdown));
    assert!(
        !live_flag.is_live(),
        "child must be dropped before wait completes"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_preempts_zero_delay_restart() {
    let supervisor = SupervisorBuilder::new()
        .restart_intensity(RestartIntensity {
            max_restarts: 8,
            within: Duration::from_secs(1),
            backoff: BackoffPolicy::None,
        })
        .child(
            ChildSpec::new("flaky", |_ctx| async move {
                Err(common::test_error("restart immediately"))
            })
            .restart(Restart::Transient),
        )
        .build()
        .expect("valid supervisor");

    let handle = supervisor.spawn();
    let mut events = handle.subscribe();

    loop {
        match recv_supervisor_event(&mut events).await {
            SupervisorEvent::ChildRestartScheduled { delay, .. } => {
                assert!(delay.is_zero(), "test requires zero-delay restart");
                handle.shutdown();
                break;
            }
            SupervisorEvent::RestartIntensityExceeded => {
                panic!("shutdown lost to zero-delay restart");
            }
            _ => {}
        }
    }

    loop {
        match recv_supervisor_event(&mut events).await {
            SupervisorEvent::SupervisorStopping => break,
            SupervisorEvent::ChildRestarted { .. } => {
                panic!("child restarted after shutdown was requested");
            }
            SupervisorEvent::RestartIntensityExceeded => {
                panic!("shutdown lost to zero-delay restart");
            }
            _ => {}
        }
    }

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
