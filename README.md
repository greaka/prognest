# Prognest

A progress library.

---
This is a very simple library to map subtask progress to an absolute range
externally. An async listener can subscribe to updates to the absolute
progress. Tasks can spawn and nest infinitely many subtasks and also
update the progress.

### Example

```rust
use prognest::Progress;

let prog = Progress::new(10000);
let mut rx = prog.subscribe();

let mut subtask = prog.allocate(8000);
subtask.set_internal(10000);

// 5000 progress / 10000 internal * 8000 alloc = 4000
subtask.advance(5000);
assert_eq!(*rx.borrow_and_update(), 4000);
```

### Design Considerations
This library was kept as simple as possible. There is no check against
overshooting allocated or global progress. There is also no way to know how
much progress a single subtask contributed so far or how far along it
already progressed.

The assumption is that a task will set up subtasks and/or
set the internal max value before starting to progress.