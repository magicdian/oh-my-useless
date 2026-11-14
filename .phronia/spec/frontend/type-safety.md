# Type Safety

> Type-safety guidance for a frontend that does not yet exist.

---

## Current State

The repository currently gets its type safety from Rust, not from TypeScript.

Examples:

* config sections are explicit Rust structs in `src/config.rs`,
* policy outputs are explicit Rust structs in `src/policy.rs`,
* sensor samples are explicit Rust structs in `src/sensor.rs`.

There is no frontend type system to document yet.

---

## Current Convention

Do not assume TypeScript is already part of the stack.

If a frontend is added later, the task must define:

* whether TypeScript is required,
* where shared UI types live,
* how runtime validation happens at backend/frontend boundaries.

Because the project already has strong Rust-side contracts, any future frontend must document how it maps onto those backend contracts instead of inventing parallel shapes ad hoc.
