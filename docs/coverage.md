# Coverage contract

The gate compares LLVM's aggregate covered and total **functions, lines, and regions** using integer arithmetic. Equality passes. Branches are measured on nightly and default to 100%, while an explicitly configured branch threshold requires nightly. Instantiations and MC/DC are not enforced. A zero-count metric is vacuous, but a report with no measured code fails.

The report must be an LLVM coverage JSON export (major version 2 or 3), contain source filenames and all required metrics, and satisfy covered <= count. Tests must pass before reporting. A failed cleanup, test run, report command, or parse cannot produce a successful coverage gate.

Each run cleans workspace coverage artifacts before instrumentation. This deliberately adds cleanup and possible recompilation cost in exchange for measuring only the current run. Test and report commands use the same Cargo target directory. Independent Lockpick processes must not run coverage concurrently in the same target directory.

Source selection follows [cargo-llvm-cov's documented defaults](https://github.com/taiki-e/cargo-llvm-cov#exclude-file-from-coverage): workspace Rust sources, excluding dependencies, build scripts, and test/example/benchmark paths. Ordinary doctests run separately and do not contribute to this measurement. Project-provided `coverage(off)` attributes also affect measurement. Lockpick's own nightly run measures all 22 production files under `src/`, with only test modules excluded by `coverage(off)`. Stable lacks that attribute, so inline test code contributes to its totals.

The integration tests launch the instrumented Lockpick binary against dependency-free temporary crates, never against the Lockpick repository. `run_lockpick` clears Cargo instrumentation variables, wrappers, and target directories, retaining `LLVM_PROFILE_FILE` only so the Lockpick child contributes to the outer measurement. The nested coverage tool chooses its own profile sink for the fixture. These invocations are bounded to 60 seconds. There is no recursive execution of Lockpick's test suite.

Deterministic tests cover schema errors, command failure, launch failure, cleanup ordering, impossible counters, zero counts, exact thresholds and large counts. Real-tool tests prove both full coverage and failure, including a passing run followed by a run that no longer calls the same unchanged function. These tests run during ordinary tests and during instrumented CI.

The audited 0.7.0 Linux baseline measured 1,208/1,311 lines (92.14%), 168/184 functions (91.30%), 1,837/1,988 regions (92.40%) and 189/224 branches (84.38%). A 100% Codecov target is an aspiration, not evidence of achieved coverage. Platform-specific code requires its corresponding platform and process/terminal failure paths remain part of the production denominator.
