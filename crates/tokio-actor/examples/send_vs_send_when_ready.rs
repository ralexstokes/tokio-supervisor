use std::{error::Error, time::Duration};

use tokio::{sync::mpsc, task::JoinHandle, time::timeout};
use tokio_actor::{
    ActorContext, ActorRunError, ActorSet, ActorSpec, Envelope, GraphBuilder, RunnableActor,
    SendError,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug)]
enum DeliveryMode {
    Send,
    SendWhenReady,
}

#[derive(Debug)]
enum SourceEvent {
    Received(String),
    Sent(String),
    SendFailed { payload: String, error: SendError },
    SendWhenReadyStarted(String),
    SendWhenReadyCompleted(String),
    SendWhenReadyFailed { payload: String, error: SendError },
}

fn describe_source_event(event: SourceEvent) -> String {
    match event {
        SourceEvent::Received(payload) => format!("source received `{payload}`"),
        SourceEvent::Sent(payload) => format!("ctx.send delivered `{payload}`"),
        SourceEvent::SendFailed { payload, error } => {
            format!("ctx.send failed for `{payload}`: {error}")
        }
        SourceEvent::SendWhenReadyStarted(payload) => {
            format!("ctx.send_when_ready started for `{payload}`")
        }
        SourceEvent::SendWhenReadyCompleted(payload) => {
            format!("ctx.send_when_ready delivered `{payload}`")
        }
        SourceEvent::SendWhenReadyFailed { payload, error } => {
            format!("ctx.send_when_ready failed for `{payload}`: {error}")
        }
    }
}

fn describe_envelope(envelope: Envelope) -> String {
    String::from_utf8_lossy(envelope.as_slice()).into_owned()
}

fn start_actor(actor: RunnableActor) -> (CancellationToken, JoinHandle<Result<(), ActorRunError>>) {
    let stop = CancellationToken::new();
    let task = tokio::spawn({
        let stop = stop.clone();
        async move { actor.run_until(stop.cancelled()).await }
    });
    (stop, task)
}

async fn stop_actor(
    stop: CancellationToken,
    task: JoinHandle<Result<(), ActorRunError>>,
) -> Result<(), Box<dyn Error>> {
    stop.cancel();
    timeout(Duration::from_secs(1), task).await???;
    Ok(())
}

fn actor(actor_set: &ActorSet, id: &str) -> RunnableActor {
    actor_set.actor(id).expect("actor exists").clone()
}

fn build_actor_set(
    mode: DeliveryMode,
    source_events_tx: mpsc::UnboundedSender<SourceEvent>,
    observed_tx: mpsc::UnboundedSender<Envelope>,
) -> Result<ActorSet, Box<dyn Error>> {
    let actor_set = GraphBuilder::new()
        .actor(ActorSpec::from_actor("source", {
            let source_events_tx = source_events_tx.clone();
            move |mut ctx: ActorContext| {
                let source_events_tx = source_events_tx.clone();
                async move {
                    while let Some(envelope) = ctx.recv().await {
                        let payload = String::from_utf8_lossy(envelope.as_slice()).into_owned();
                        source_events_tx
                            .send(SourceEvent::Received(payload.clone()))
                            .expect("observer alive");

                        match mode {
                            DeliveryMode::Send => match ctx.send("sink", envelope).await {
                                Ok(()) => {
                                    source_events_tx
                                        .send(SourceEvent::Sent(payload))
                                        .expect("observer alive");
                                }
                                Err(error) => {
                                    source_events_tx
                                        .send(SourceEvent::SendFailed { payload, error })
                                        .expect("observer alive");
                                }
                            },
                            DeliveryMode::SendWhenReady => {
                                source_events_tx
                                    .send(SourceEvent::SendWhenReadyStarted(payload.clone()))
                                    .expect("observer alive");
                                match ctx.send_when_ready("sink", envelope).await {
                                    Ok(()) => {
                                        source_events_tx
                                            .send(SourceEvent::SendWhenReadyCompleted(payload))
                                            .expect("observer alive");
                                    }
                                    Err(error) => {
                                        source_events_tx
                                            .send(SourceEvent::SendWhenReadyFailed {
                                                payload,
                                                error,
                                            })
                                            .expect("observer alive");
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }
            }
        }))
        .actor(ActorSpec::from_actor("sink", {
            let observed_tx = observed_tx.clone();
            move |mut ctx: ActorContext| {
                let observed_tx = observed_tx.clone();
                async move {
                    while let Some(envelope) = ctx.recv().await {
                        observed_tx.send(envelope).expect("observer alive");
                    }
                    Ok(())
                }
            }
        }))
        .link("source", "sink")
        .ingress("requests", "source")
        .build()?
        .into_actor_set()?;

    Ok(actor_set)
}

async fn next_source_event(
    source_events_rx: &mut mpsc::UnboundedReceiver<SourceEvent>,
) -> Result<SourceEvent, Box<dyn Error>> {
    let event = timeout(Duration::from_secs(1), source_events_rx.recv()).await?;
    event.ok_or_else(|| std::io::Error::other("source event channel closed").into())
}

async fn next_observed(
    observed_rx: &mut mpsc::UnboundedReceiver<Envelope>,
) -> Result<Envelope, Box<dyn Error>> {
    let envelope = timeout(Duration::from_secs(1), observed_rx.recv()).await?;
    envelope.ok_or_else(|| std::io::Error::other("observed channel closed").into())
}

async fn run_send_demo() -> Result<(), Box<dyn Error>> {
    let (source_events_tx, mut source_events_rx) = mpsc::unbounded_channel();
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let actor_set = build_actor_set(DeliveryMode::Send, source_events_tx, observed_tx)?;
    let source = actor(&actor_set, "source");
    let sink = actor(&actor_set, "sink");
    let mut ingress = actor_set.ingress("requests").expect("ingress exists");

    let (source_stop, source_task) = start_actor(source);
    let (sink_stop, sink_task) = start_actor(sink);

    ingress.wait_for_binding().await;
    ingress.send("before-stop").await?;

    println!("== ctx.send ==");
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );
    println!(
        "sink observed: {}",
        describe_envelope(next_observed(&mut observed_rx).await?)
    );

    stop_actor(sink_stop, sink_task).await?;
    println!("sink cancelled");

    ingress.send("after-stop").await?;
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );

    stop_actor(source_stop, source_task).await?;
    Ok(())
}

async fn run_send_when_ready_demo() -> Result<(), Box<dyn Error>> {
    let (source_events_tx, mut source_events_rx) = mpsc::unbounded_channel();
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let actor_set = build_actor_set(DeliveryMode::SendWhenReady, source_events_tx, observed_tx)?;
    let source = actor(&actor_set, "source");
    let sink = actor(&actor_set, "sink");
    let mut ingress = actor_set.ingress("requests").expect("ingress exists");

    let (source_stop, source_task) = start_actor(source);
    let (sink_stop, sink_task) = start_actor(sink.clone());

    ingress.wait_for_binding().await;
    ingress.send("before-stop").await?;

    println!("== ctx.send_when_ready ==");
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );
    println!(
        "sink observed: {}",
        describe_envelope(next_observed(&mut observed_rx).await?)
    );

    stop_actor(sink_stop, sink_task).await?;
    println!("sink cancelled");

    ingress.send("after-stop").await?;
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );
    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );

    assert!(
        timeout(Duration::from_millis(200), observed_rx.recv())
            .await
            .is_err(),
        "send_when_ready should wait while the sink is down"
    );
    assert!(
        timeout(Duration::from_millis(200), source_events_rx.recv())
            .await
            .is_err(),
        "send_when_ready should not report completion before the sink restarts"
    );
    println!("no delivery yet; source is waiting for sink to restart");

    let (sink_stop, sink_task) = start_actor(sink);
    println!("sink restarted");

    println!(
        "event: {}",
        describe_source_event(next_source_event(&mut source_events_rx).await?)
    );
    println!(
        "sink observed: {}",
        describe_envelope(next_observed(&mut observed_rx).await?)
    );

    stop_actor(sink_stop, sink_task).await?;
    stop_actor(source_stop, source_task).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    run_send_demo().await?;
    println!();
    run_send_when_ready_demo().await?;
    Ok(())
}
