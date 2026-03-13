use std::{
    error::Error,
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{broadcast, mpsc, watch},
    time::timeout,
};
use tokio_actor::{ActorContext, ActorSpec, Envelope, GraphBuilder};
use tokio_otp::SupervisedActors;
use tokio_supervisor::{
    BackoffPolicy, ChildSnapshot, ChildStateView, ExitStatusView, RestartIntensity, Strategy,
    SupervisorBuilder, SupervisorEvent, SupervisorExit, SupervisorHandle, SupervisorSnapshot,
    SupervisorStateView,
};

fn actor_error(message: &'static str) -> Box<dyn Error + Send + Sync + 'static> {
    Box::new(io::Error::other(message))
}

fn describe_child_state(state: ChildStateView) -> &'static str {
    match state {
        ChildStateView::Starting => "starting",
        ChildStateView::Running => "running",
        ChildStateView::Stopping => "stopping",
        ChildStateView::Stopped => "stopped",
    }
}

fn describe_last_exit(last_exit: &Option<ExitStatusView>) -> String {
    match last_exit {
        None => "none".to_owned(),
        Some(ExitStatusView::Completed) => "completed".to_owned(),
        Some(ExitStatusView::Failed(error)) => format!("failed({error})"),
        Some(ExitStatusView::Panicked) => "panicked".to_owned(),
        Some(ExitStatusView::Aborted) => "aborted".to_owned(),
    }
}

fn print_snapshot(label: &str, snapshot: &SupervisorSnapshot) {
    println!(
        "{label}: supervisor state={:?} last_exit={:?} strategy={:?}",
        snapshot.state, snapshot.last_exit, snapshot.strategy
    );

    for child in &snapshot.children {
        print_child_snapshot(child);
    }
}

fn print_child_snapshot(child: &ChildSnapshot) {
    println!(
        "  child={} generation={} state={} restarts={} next_restart_in={:?} last_exit={}",
        child.id,
        child.generation,
        describe_child_state(child.state),
        child.restart_count,
        child.next_restart_in,
        describe_last_exit(&child.last_exit)
    );
}

fn print_event_and_snapshot(label: &str, event: &SupervisorEvent, snapshot: &SupervisorSnapshot) {
    println!("{label}: {event:?}");
    print_snapshot("snapshot after event", snapshot);
}

async fn next_observed(
    observed_rx: &mut mpsc::UnboundedReceiver<String>,
) -> Result<String, Box<dyn Error>> {
    let observed = timeout(Duration::from_secs(2), observed_rx.recv()).await?;
    observed.ok_or_else(|| io::Error::other("observed channel closed").into())
}

async fn wait_for_snapshot(
    snapshots: &mut watch::Receiver<SupervisorSnapshot>,
    ready: impl Fn(&SupervisorSnapshot) -> bool,
) -> Result<SupervisorSnapshot, Box<dyn Error>> {
    if ready(&snapshots.borrow()) {
        return Ok(snapshots.borrow().clone());
    }

    loop {
        timeout(Duration::from_secs(2), snapshots.changed()).await??;
        if ready(&snapshots.borrow()) {
            return Ok(snapshots.borrow().clone());
        }
    }
}

fn drain_pending_events(
    handle: &SupervisorHandle,
    events: &mut broadcast::Receiver<SupervisorEvent>,
) -> Result<(), Box<dyn Error>> {
    loop {
        match events.try_recv() {
            Ok(event) => {
                let snapshot = handle.snapshot();
                print_event_and_snapshot("pending event", &event, &snapshot);
            }
            Err(broadcast::error::TryRecvError::Empty)
            | Err(broadcast::error::TryRecvError::Closed) => return Ok(()),
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                return Err(io::Error::other(format!(
                    "event observer lagged and skipped {skipped} events"
                ))
                .into());
            }
        }
    }
}

async fn observe_until(
    handle: &SupervisorHandle,
    events: &mut broadcast::Receiver<SupervisorEvent>,
    ready: impl Fn(&SupervisorEvent, &SupervisorSnapshot) -> bool,
) -> Result<(), Box<dyn Error>> {
    loop {
        let event = timeout(Duration::from_secs(2), events.recv()).await??;
        let snapshot = handle.snapshot();
        print_event_and_snapshot("event", &event, &snapshot);

        if ready(&event, &snapshot) {
            return Ok(());
        }
    }
}

fn child_is_running(snapshot: &SupervisorSnapshot, id: &str) -> bool {
    snapshot
        .child(id)
        .is_some_and(|child| child.state == ChildStateView::Running)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let worker_runs = Arc::new(AtomicUsize::new(0));

    let graph = GraphBuilder::new()
        .actor(ActorSpec::from_actor(
            "frontend",
            |mut ctx: ActorContext| async move {
                while let Some(envelope) = ctx.recv().await {
                    ctx.send_when_ready("worker", envelope).await?;
                }
                Ok(())
            },
        ))
        .actor(ActorSpec::from_actor("worker", {
            let observed_tx = observed_tx.clone();
            let worker_runs = Arc::clone(&worker_runs);
            move |mut ctx: ActorContext| {
                let observed_tx = observed_tx.clone();
                let worker_runs = Arc::clone(&worker_runs);
                async move {
                    let run = worker_runs.fetch_add(1, Ordering::SeqCst) + 1;
                    println!("worker actor run {run} started");

                    while let Some(envelope) = ctx.recv().await {
                        let payload = String::from_utf8_lossy(envelope.as_slice()).into_owned();
                        if payload == "fail-worker" {
                            println!("worker actor run {run} failing on `{payload}`");
                            return Err(actor_error("simulated worker failure"));
                        }

                        let observed = format!("worker run {run} observed `{payload}`");
                        println!("{observed}");
                        observed_tx.send(observed).expect("observer alive");
                    }

                    println!("worker actor run {run} stopping");
                    Ok(())
                }
            }
        }))
        .link("frontend", "worker")
        .ingress("requests", "frontend")
        .build()?;

    let (supervisor, mut ingresses) = SupervisedActors::new(graph)?
        .actor_restart_intensity(
            "worker",
            RestartIntensity::new(2, Duration::from_secs(1))
                .with_backoff(BackoffPolicy::Fixed(Duration::from_millis(200))),
        )
        .build_supervisor(SupervisorBuilder::new().strategy(Strategy::OneForOne))?;

    let handle = supervisor.spawn();
    let mut events = handle.subscribe();
    let mut snapshots = handle.subscribe_snapshots();
    let mut ingress = ingresses.remove("requests").expect("ingress exists");

    print_snapshot("snapshot immediately after spawn", &handle.snapshot());

    let steady_state = wait_for_snapshot(&mut snapshots, |snapshot| {
        child_is_running(snapshot, "frontend") && child_is_running(snapshot, "worker")
    })
    .await?;
    print_snapshot("snapshot once both children are running", &steady_state);
    drain_pending_events(&handle, &mut events)?;

    ingress.wait_for_binding().await;
    ingress.send(Envelope::from_static(b"hello")).await?;
    println!(
        "observed message: {}",
        next_observed(&mut observed_rx).await?
    );
    print_snapshot("snapshot after a normal delivery", &handle.snapshot());

    println!("sending `fail-worker` to trigger a restart");
    ingress.send(Envelope::from_static(b"fail-worker")).await?;
    observe_until(&handle, &mut events, |event, snapshot| {
        matches!(
            event,
            SupervisorEvent::ChildRestartScheduled { id, .. } if id == "worker"
        ) && snapshot.child("worker").is_some_and(|child| {
            child.state == ChildStateView::Stopped
                && child.next_restart_in.is_some()
                && matches!(child.last_exit, Some(ExitStatusView::Failed(_)))
        })
    })
    .await?;

    println!("sending `after-restart` while the worker is still down");
    ingress
        .send(Envelope::from_static(b"after-restart"))
        .await?;
    observe_until(&handle, &mut events, |event, snapshot| {
        matches!(
            event,
            SupervisorEvent::ChildRestarted { id, .. } if id == "worker"
        ) && snapshot
            .child("worker")
            .is_some_and(|child| child.generation == 1 && child.state == ChildStateView::Running)
    })
    .await?;

    println!(
        "observed message after restart: {}",
        next_observed(&mut observed_rx).await?
    );
    print_snapshot("snapshot after post-restart delivery", &handle.snapshot());

    println!("requesting supervisor shutdown");
    handle.shutdown();
    observe_until(&handle, &mut events, |event, snapshot| {
        matches!(event, SupervisorEvent::SupervisorStopped)
            && snapshot.state == SupervisorStateView::Stopped
    })
    .await?;

    let exit = handle.wait().await?;
    assert_eq!(exit, SupervisorExit::Shutdown);
    println!("supervisor exited with {exit:?}");
    print_snapshot("final snapshot", &handle.snapshot());

    Ok(())
}
