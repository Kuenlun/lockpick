# Coverage

Lockpick enforces aggregate function, line and region coverage, plus branch coverage on nightly. Instantiations and MC/DC are not enforced. See the [README](../README.md#configuration) for configuration.

Lockpick's CI requires 100% in all four metrics separately on Linux, macOS and Windows.

Source selection follows [cargo-llvm-cov's defaults](https://github.com/taiki-e/cargo-llvm-cov#exclude-file-from-coverage). Doctests run separately and do not contribute to coverage. Inline test code is counted unless excluded with `coverage(off)`. Lockpick excludes its test modules on nightly, where that attribute is available.

Concurrent coverage runs must use separate Cargo target directories.
