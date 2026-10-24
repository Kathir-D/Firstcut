//! Burst batching: turning capture-ordered photos into batches the user culls one at a time.
//!
//! Ported verbatim from v1 `firstcut_core::order` (as module [`order`]) and `::batch` (as module
//! [`batch`]) at the M1 port, plus the committed regression test in `tests/batching.rs`. The `Photo`
//! view trait, the ids and the fixture/exiftool adapters' id math are re-exported from
//! `firstcut-types`, where they live for `firstcut-meta` — which implements the trait — without a
//! meta↔batching edge (docs/v2-plan.md §4.1).
//!
//! Everything is deterministic: same input, same batches, always (todo.md §5.3 step 6). Owner:
//! core-batch (see docs/contracts/batching.md).

pub mod batch;
pub mod order;
