# tokio-supervisor

Erlang/OTP-style supervisor trees in the tokio ecosystem

## Development

Use the flake for both local tooling and CI:

```sh
nix develop
just ci
```

GitHub Actions runs `nix flake check --no-update-lock-file --print-build-logs`, and the Rust-specific checks in the flake turn on automatically once the repo has a root `Cargo.toml`.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
