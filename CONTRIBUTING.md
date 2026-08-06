# Contributing

Tea is in an active `0.1.x` iteration and its architecture is not yet stable.
External pull requests are not currently accepted. Please use a [GitHub
Issue](https://github.com/tea-hq/tea-rs/issues) for ideas, suggestions, and
questions so the scope and compatibility impact can be discussed first.

## Development

Keep dependencies flowing toward the provider-neutral runtime contracts. Keep
provider-specific types inside provider adapters and avoid putting product
behavior into the kernel.

Use deterministic fake providers and tools in tests. Tests must not require
paid APIs, network access, credentials, or real user data.

## Validation

Run the checks relevant to your change before opening a pull request:

```bash
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

For a focused change, a package-level test is sufficient during development.
Please include the commands you ran and any platform-specific limitations in
the pull request description.

## Pull requests

Pull requests are currently limited to changes explicitly invited by the
maintainers. Do not open an unsolicited external pull request during the
`0.1.x` iteration. For an invited change, keep it focused, include tests for
behavior changes, and update public documentation when user-facing behavior
changes. Do not commit secrets, generated credentials, private data, or
unredacted provider payloads.

Use clear commit messages such as:

```text
feat(protocol): add canonical agent events
fix(kernel): preserve tool result order
```

## License

By contributing, you agree that your contributions are provided under the
Apache License, Version 2.0.
