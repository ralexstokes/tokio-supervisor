use std::{error::Error, io, str, time::Duration};

use tokio::{sync::mpsc, time::timeout};
use tokio_actor::{ActorContext, ActorSpec, Envelope, GraphBuilder};
use tokio_otp::{DynamicActorOptions, SupervisedActors};
use tokio_supervisor::{Strategy, SupervisorBuilder, SupervisorExit};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();

    let graph = GraphBuilder::new()
        .actor(ActorSpec::from_actor(
            "frontend",
            |mut ctx: ActorContext| async move {
                while let Some(envelope) = ctx.recv().await {
                    println!(
                        "frontend forwarding `{}` to dynamic-worker",
                        str::from_utf8(envelope.as_slice()).expect("valid message"),
                    );
                    ctx.send_dynamic_when_ready("dynamic-worker", envelope)
                        .await?;
                }
                Ok(())
            },
        ))
        .actor(ActorSpec::from_actor("sink", {
            let observed_tx = observed_tx.clone();
            move |mut ctx: ActorContext| {
                let observed_tx = observed_tx.clone();
                async move {
                    while let Some(envelope) = ctx.recv().await {
                        observed_tx.send(envelope).expect("receiver alive");
                    }
                    Ok(())
                }
            }
        }))
        .ingress("requests", "frontend")
        .build()?;

    let runtime = SupervisedActors::new(graph)?
        .build_runtime(SupervisorBuilder::new().strategy(Strategy::OneForOne))?;
    let handle = runtime.spawn();
    let mut ingress = handle.ingress("requests").expect("ingress exists");

    let mut added_ref = handle
        .add_actor(
            ActorSpec::from_actor("dynamic-worker", |mut ctx: ActorContext| async move {
                while let Some(envelope) = ctx.recv().await {
                    println!(
                        "dynamic-worker forwarding `{}` to sink",
                        str::from_utf8(envelope.as_slice()).expect("valid message"),
                    );
                    ctx.send("sink", envelope).await?;
                }
                Ok(())
            }),
            DynamicActorOptions {
                peer_ids: vec!["sink".to_owned()],
                ..DynamicActorOptions::default()
            },
        )
        .await?;

    let mut looked_up_ref = handle
        .actor_ref("dynamic-worker")
        .expect("dynamic actor is registered");

    added_ref.wait_for_binding().await;
    looked_up_ref.wait_for_binding().await;
    ingress.wait_for_binding().await;

    ingress.send(Envelope::from_static(b"hello")).await?;
    print_observed(&mut observed_rx, "through ingress").await?;

    looked_up_ref
        .send(Envelope::from_static(b"sent-directly"))
        .await?;
    print_observed(&mut observed_rx, "through actor_ref").await?;

    handle.remove_actor("dynamic-worker").await?;
    assert!(handle.actor_ref("dynamic-worker").is_none());
    println!("dynamic-worker removed");

    let exit = handle.shutdown_and_wait().await?;
    assert_eq!(exit, SupervisorExit::Shutdown);
    println!("runtime exited with {exit:?}");

    Ok(())
}

async fn print_observed(
    observed_rx: &mut mpsc::UnboundedReceiver<Envelope>,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    let envelope = timeout(Duration::from_secs(1), observed_rx.recv())
        .await?
        .ok_or_else(|| io::Error::other("observation channel closed"))?;
    println!(
        "sink observed {label}: `{}`",
        str::from_utf8(envelope.as_slice()).expect("valid message"),
    );
    Ok(())
}
