use tea_control::CancellationScope;

#[tokio::test(flavor = "current_thread")]
async fn shared_scope_propagates_to_children() {
    let root = CancellationScope::new();
    let first = root.child();
    let second = root.child();

    root.cancel();
    first.cancelled().await;
    second.cancelled().await;

    assert!(first.is_cancelled());
    assert!(second.is_cancelled());
}

#[tokio::test(flavor = "current_thread")]
async fn child_cancellation_does_not_cancel_parent_or_sibling() {
    let root = CancellationScope::new();
    let first = root.child();
    let second = root.child();

    first.cancel();
    first.cancelled().await;

    assert!(!root.is_cancelled());
    assert!(!second.is_cancelled());
}

#[test]
fn shared_scope_is_send_sync_and_default_is_pending() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CancellationScope>();
    assert!(!CancellationScope::default().is_cancelled());
}
