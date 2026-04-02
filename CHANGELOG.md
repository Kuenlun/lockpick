# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.3](https://github.com/Kuenlun/lockpick/compare/v0.3.2...v0.3.3) - 2026-04-02

### Other

- fix tag parsing in release-plz workflow ([#12](https://github.com/Kuenlun/lockpick/pull/12))

## [0.3.2](https://github.com/Kuenlun/lockpick/compare/v0.3.1...v0.3.2) - 2026-04-02

### Other

- fix release-plz PRs not triggering CI workflows ([#11](https://github.com/Kuenlun/lockpick/pull/11))
- fix release binary uploads by merging artifact workflow into release-plz ([#9](https://github.com/Kuenlun/lockpick/pull/9))

## [0.3.1](https://github.com/Kuenlun/lockpick/compare/v0.3.0...v0.3.1) - 2026-04-02

### Added

- *(cli)* replace opt-out flags with composable `--skip` and add coverage support ([#7](https://github.com/Kuenlun/lockpick/pull/7))

### Other

- add GitHub Actions workflow for releasing compiled binaries ([#6](https://github.com/Kuenlun/lockpick/pull/6))

## [0.3.0](https://github.com/Kuenlun/lockpick/releases/tag/v0.3.0) - 2026-04-02

### Added

- *(core)* implement core CLI logic and parallel cargo check runner ([#2](https://github.com/Kuenlun/lockpick/pull/2))
- initialize lockpick Rust project and CI infrastructure ([#1](https://github.com/Kuenlun/lockpick/pull/1))

### Other

- configure workflows and align CI with repository conventions ([#4](https://github.com/Kuenlun/lockpick/pull/4))
- configure release automation, pre-commit hooks, and CI improvements ([#3](https://github.com/Kuenlun/lockpick/pull/3))
