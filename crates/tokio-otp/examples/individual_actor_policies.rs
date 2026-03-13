use std::{
    collections::HashMap,
    error::Error,
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{broadcast, mpsc},
    time::{sleep, timeout},
};
use tokio_actor::{ActorContext, ActorSpec, Envelope, GraphBuilder};
use tokio_otp::SupervisedActors;
use tokio_supervisor::{
    BackoffPolicy, ChildSpec, ExitStatusView, Restart, RestartIntensity, ShutdownPolicy, Strategy,
    SupervisorBuilder, SupervisorEvent, SupervisorExit,
};

fn example_error(message: &'static str) -> Box<dyn Error + Send + Sync + 'static> {
    Box::new(io::Error::other(message))
}

fn into_children_map(children: Vec<ChildSpec>) -> HashMap<String, ChildSpec> {
    children
        .into_iter()
        .map(|child| (child.id().to_owned(), child))
        .collect()
}

fn take_child(children: &mut HashMap<String, ChildSpec>, id: &str) -> ChildSpec {
    children.remove(id).expect("child exists")
}

async fn next_observed(
    observed_rx: &mut mpsc::UnboundedReceiver<String>,
) -> Result<String, Box<dyn Error>> {
    let observed = timeout(Duration::from_secs(1), observed_rx.recv()).await?;
    observed.ok_or_else(|| io::Error::other("observed channel closed").into())
}

#[derive(Default)]
struct EventState {
    generator_failed: bool,
    worker_restart_scheduled: bool,
    worker_restarted: bool,
}

fn observe_event(state: &mut EventState, event: &SupervisorEvent) -> Result<(), Box<dyn Error>> {
    match event {
        SupervisorEvent::ChildExited {
            id,
            generation,
            status: ExitStatusView::Failed(error),
        } if id == "generator" => {
            state.generator_failed = true;
            println!("generator generation {generation} failed once and stays down: {error}");
        }
        SupervisorEvent::ChildRestartScheduled {
            id,
            generation,
            delay,
        } if id == "worker" => {
            state.worker_restart_scheduled = true;
            println!("worker generation {generation} is scheduled for restart after {delay:?}");
        }
        SupervisorEvent::ChildRestarted {
            id,
            old_generation,
            new_generation,
        } if id == "worker" => {
            state.worker_restarted = true;
            println!(
                "worker restarted from generation {old_generation} to generation {new_generation}"
            );
        }
        SupervisorEvent::RestartIntensityExceeded => {
            return Err(io::Error::other("unexpected restart intensity failure").into());
        }
        _ => {}
    }

    Ok(())
}

async fn wait_for_events(
    events: &mut broadcast::Receiver<SupervisorEvent>,
    state: &mut EventState,
    ready: impl Fn(&EventState) -> bool,
) -> Result<(), Box<dyn Error>> {
    while !ready(state) {
        let event = timeout(Duration::from_secs(2), events.recv()).await??;
        println!("event: {event:?}");
        observe_event(state, &event)?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let frontend_runs = Arc::new(AtomicUsize::new(0));
    let generator_runs = Arc::new(AtomicUsize::new(0));
    let worker_runs = Arc::new(AtomicUsize::new(0));

    let graph = GraphBuilder::new()
        .actor(ActorSpec::from_actor("frontend", {
            let frontend_runs = Arc::clone(&frontend_runs);
            move |mut ctx: ActorContext| {
                let frontend_runs = Arc::clone(&frontend_runs);
                async move {
                    let run = frontend_runs.fetch_add(1, Ordering::SeqCst) + 1;
                    println!("frontend actor run {run} started");

                    while let Some(envelope) = ctx.recv().await {
                        let payload = String::from_utf8_lossy(envelope.as_slice()).into_owned();
                        println!("frontend actor run {run} forwarding `{payload}`");
                        ctx.send_when_ready("worker", envelope).await?;
                        println!("frontend actor run {run} delivered `{payload}`");
                    }

                    println!("frontend actor run {run} stopping");
                    Ok(())
                }
            }
        }))
        .actor(ActorSpec::from_actor("generator", {
            let generator_runs = Arc::clone(&generator_runs);
            move |ctx: ActorContext| {
                let generator_runs = Arc::clone(&generator_runs);
                async move {
                    let run = generator_runs.fetch_add(1, Ordering::SeqCst) + 1;
                    println!("generator actor run {run} started");

                    ctx.send_when_ready("worker", Envelope::from_static(b"generated-once"))
                        .await?;
                    sleep(Duration::from_millis(50)).await;

                    println!("generator actor run {run} failing");
                    Err(example_error("simulated one-shot generator failure"))
                }
            }
        }))
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
                            return Err(example_error("simulated worker failure"));
                        }

                        println!("worker actor run {run} observed `{payload}`");
                        observed_tx
                            .send(format!("worker run {run} observed `{payload}`"))
                            .expect("observer alive");
                    }

                    println!("worker actor run {run} stopping");
                    Ok(())
                }
            }
        }))
        .link("frontend", "worker")
        .link("generator", "worker")
        .ingress("requests", "frontend")
        .build()?;

    let (children, mut ingresses) = SupervisedActors::new(graph)?.build()?;
    let mut children = into_children_map(children);

    let worker = take_child(&mut children, "worker")
        .restart(Restart::Transient)
        .restart_intensity(
            RestartIntensity::new(2, Duration::from_secs(1))
                .with_backoff(BackoffPolicy::Fixed(Duration::from_millis(100))),
        )
        .shutdown(ShutdownPolicy::cooperative_then_abort(
            Duration::from_millis(250),
        ));
    let frontend = take_child(&mut children, "frontend")
        .restart(Restart::Permanent)
        .shutdown(ShutdownPolicy::cooperative(Duration::from_millis(250)));
    let generator = take_child(&mut children, "generator")
        .restart(Restart::Temporary)
        .shutdown(ShutdownPolicy::abort());

    let supervisor = SupervisorBuilder::new()
        .strategy(Strategy::OneForOne)
        .child(worker)
        .child(frontend)
        .child(generator)
        .build()?;

    let handle = supervisor.spawn();
    let mut events = handle.subscribe();
    let mut ingress = ingresses.remove("requests").expect("ingress exists");

    ingress.wait_for_binding().await;

    println!("first delivery: {}", next_observed(&mut observed_rx).await?);

    ingress.send(Envelope::from_static(b"fail-worker")).await?;
    let mut event_state = EventState::default();
    wait_for_events(&mut events, &mut event_state, |state| {
        state.worker_restart_scheduled
    })
    .await?;

    ingress
        .send(Envelope::from_static(b"after-restart"))
        .await?;
    wait_for_events(&mut events, &mut event_state, |state| {
        state.generator_failed && state.worker_restarted
    })
    .await?;

    println!(
        "delivery after worker restart: {}",
        next_observed(&mut observed_rx).await?
    );

    assert_eq!(frontend_runs.load(Ordering::SeqCst), 1);
    assert_eq!(generator_runs.load(Ordering::SeqCst), 1);
    assert!(worker_runs.load(Ordering::SeqCst) >= 2);

    println!(
        "run counts: frontend={}, generator={}, worker={}",
        frontend_runs.load(Ordering::SeqCst),
        generator_runs.load(Ordering::SeqCst),
        worker_runs.load(Ordering::SeqCst)
    );

    let exit = handle.shutdown_and_wait().await?;
    assert_eq!(exit, SupervisorExit::Shutdown);
    println!("supervisor exited with {exit:?}");

    Ok(())
}
