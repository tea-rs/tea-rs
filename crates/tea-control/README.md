# tea-control

Shared operation-control primitives for `tea-rs`.

`CancellationScope` is the project-owned cooperative cancellation tree used by model providers, tool executors, and runtime operations. It wraps Tokio-util internally while exposing no Tokio token, channel, task, or runtime handle.

```rust
use tea_control::CancellationScope;

let root = CancellationScope::new();
let child = root.child();
root.cancel();
assert!(child.is_cancelled());
```

Cancellation is idempotent. Parent cancellation propagates to children; child cancellation does not cancel its parent or siblings. Library code remains responsible for awaiting owned cleanup before reporting terminal cancellation.

This crate provides cancellation primitives only; it does not create a runtime or spawn tasks.
