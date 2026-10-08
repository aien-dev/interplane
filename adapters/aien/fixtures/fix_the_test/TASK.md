# Task: fix the failing test

task_id: fix-the-test/clamp-1
test_command: make test
target_file: src/clamp.c

`make test` exits non-zero (GNU make reports 2 for a failing recipe) because `clamp()` returns `lo` where it must return `hi` when the value is
above the range. Fix `src/clamp.c` (replace the whole file). The test must not be edited.

The scripted run proposes `solution/clamp.c` as the whole new `src/clamp.c`. `solution/` is
harness material: it is not copied into the task workspace.
