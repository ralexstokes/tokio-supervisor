# Tokio Supervisor Remaining Work

This document carries forward the parts of the old handoff `SPEC.md` that are
not yet reflected by the codebase at commit
`2fe64feff2d853c570569f899474844740e7077b`.

## Hardening and coverage

The core runtime is implemented and tested, but there is still room for targeted hardening:

* direct coverage for `ShutdownMode::Cooperative` returning `SupervisorError::ShutdownTimedOut`
* direct coverage for exponential backoff behavior
* broader event-surface verification
* general race and failure-path hardening

## Out of scope

* readiness protocol
* health probes

* required vs optional children
* distributed supervision
* actor runtime or mailbox abstractions

## Notes

Some sections of the old `SPEC.md` were implementation planning material rather than current-state specification:

* implementation order
* dependency recommendations
* suggested file sketches
* broad testing matrix beyond what now exists

Those sections were intentionally not copied into `SPEC_2fe64fe.md`.
