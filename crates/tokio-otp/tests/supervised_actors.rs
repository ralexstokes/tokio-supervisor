use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};
use tokio_actor::{ActorContext, ActorSpec, BoxError, Envelope, GraphBuilder};
use tokio_otp::{BuildError, SupervisedActors};
use tokio_supervisor::{Restart, Strategy, SupervisorBuilder};

fn oneshot_slot<T>(tx: oneshot::Sender<T>) -> Arc<Mutex<Option<oneshot::Sender<T>>>> {
    Arc::new(Mutex::new(Some(tx)))
}

fn send_once<T>(slot: &Arc<Mutex<Option<oneshot::Sender<T>>>>, value: T) {
    if let Some(tx) = slot.lock().expect("mutex not poisoned").take() {
        let _ = tx.send(value);
    }
}

#[tokio::test]
async fn supervised_actors_restart_only_the_failed_actor() {
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let frontend_starts = Arc::new(AtomicUsize::new(0));
    let worker_starts = Arc::new(AtomicUsize::new(0));
    let (failed_tx, failed_rx) = oneshot::channel();
    let failed_slot = oneshot_slot(failed_tx);

    let graph = GraphBuilder::new()
        .actor(ActorSpec::from_actor("frontend", {
            let frontend_starts = Arc::clone(&frontend_starts);
            move |mut ctx: ActorContext| {
                frontend_starts.fetch_add(1, Ordering::SeqCst);
                async move {
                    while let Some(envelope) = ctx.recv().await {
                        ctx.send_when_ready("worker", envelope).await?;
                    }
                    Ok(())
                }
            }
        }))
        .actor(ActorSpec::from_actor("worker", {
            let observed_tx = observed_tx.clone();
            let worker_starts = Arc::clone(&worker_starts);
            let failed_slot = Arc::clone(&failed_slot);
            move |mut ctx: ActorContext| {
                let observed_tx = observed_tx.clone();
                let failed_slot = Arc::clone(&failed_slot);
                let run = worker_starts.fetch_add(1, Ordering::SeqCst);
                async move {
                    while let Some(envelope) = ctx.recv().await {
                        observed_tx.send(envelope.clone()).expect("receiver alive");
                        if run == 0 {
                            send_once(&failed_slot, ());
                            return Err::<(), BoxError>(Box::new(io::Error::other("boom")));
                        }
                    }
                    Ok(())
                }
            }
        }))
        .link("frontend", "worker")
        .ingress("requests", "frontend")
        .build()
        .expect("valid graph");

    let (supervisor, mut ingresses) = SupervisedActors::new(graph)
        .expect("graph decomposes")
        .restart(Restart::Transient)
        .build_supervisor(SupervisorBuilder::new().strategy(Strategy::OneForOne))
        .expect("supervisor builds");

    let ingress = ingresses.remove("requests").expect("ingress exists");
    let handle = supervisor.spawn();
    let mut ingress = ingress;

    ingress.wait_for_binding().await;
    ingress
        .send(Envelope::from_static(b"first"))
        .await
        .expect("frontend accepts the first message");
    let first = timeout(Duration::from_secs(1), observed_rx.recv())
        .await
        .expect("worker saw the first message")
        .expect("worker forwarded the first message");
    assert_eq!(first.as_slice(), b"first");

    timeout(Duration::from_secs(1), failed_rx)
        .await
        .expect("worker failed on the first run")
        .expect("worker failure signal received");

    ingress
        .send(Envelope::from_static(b"second"))
        .await
        .expect("frontend accepts the second message");
    let second = timeout(Duration::from_secs(1), observed_rx.recv())
        .await
        .expect("worker saw the second message after restart")
        .expect("worker forwarded the second message");
    assert_eq!(second.as_slice(), b"second");

    assert_eq!(frontend_starts.load(Ordering::SeqCst), 1);
    assert!(worker_starts.load(Ordering::SeqCst) >= 2);

    handle
        .shutdown_and_wait()
        .await
        .expect("supervisor shut down cleanly");
}

#[test]
fn supervised_actors_reject_unknown_actor_overrides() {
    let graph = GraphBuilder::new()
        .actor(ActorSpec::from_actor(
            "worker",
            |_ctx: ActorContext| async move { Ok(()) },
        ))
        .build()
        .expect("valid graph");

    let result = SupervisedActors::new(graph)
        .expect("graph decomposes")
        .actor_restart("missing", Restart::Permanent)
        .build();

    assert!(matches!(
        result,
        Err(BuildError::UnknownActor { actor_id }) if actor_id == "missing"
    ));
}
