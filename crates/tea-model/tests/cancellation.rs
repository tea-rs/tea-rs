use tea_control::CancellationScope;
use tea_model::ModelCancellation;

#[tokio::test(flavor = "current_thread")]
async fn cancellation_propagates_to_clones_and_children() {
    let parent = ModelCancellation::new();
    let clone = parent.clone();
    let child = parent.child();

    assert!(!parent.is_cancelled());
    assert!(!clone.is_cancelled());
    assert!(!child.is_cancelled());

    parent.cancel();
    parent.cancelled().await;
    clone.cancelled().await;
    child.cancelled().await;

    assert!(parent.is_cancelled());
    assert!(clone.is_cancelled());
    assert!(child.is_cancelled());
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_child_does_not_cancel_parent() {
    let parent = ModelCancellation::new();
    let child = parent.child();

    child.cancel();
    child.cancelled().await;

    assert!(child.is_cancelled());
    assert!(!parent.is_cancelled());
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_future_wakes_without_spawning() {
    let cancellation = ModelCancellation::new();
    let canceller = cancellation.clone();

    tokio::join!(cancellation.cancelled(), async move {
        canceller.cancel();
    });

    assert!(cancellation.is_cancelled());
}

#[tokio::test(flavor = "current_thread")]
async fn already_cancelled_scope_resolves_immediately() {
    let cancellation = ModelCancellation::new();
    cancellation.cancel();
    cancellation.cancelled().await;
    assert!(cancellation.is_cancelled());
}

#[test]
fn cancellation_is_shared_alias_send_sync_and_default_is_pending() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ModelCancellation>();
    let shared: CancellationScope = ModelCancellation::default();
    assert!(!shared.is_cancelled());
}
