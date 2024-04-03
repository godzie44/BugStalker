# Changelog

All notable changes to this project will be documented in this file.

# [?.?.?] Unreleased

### Added

### Changed

### Fixed

### Deprecated

### Breaking changes

---

# [0.1.4] April 3 2024

### Changed
- console: history hints now have better highlighting (grey instead of bolt)

### Fixed
- console: now sub commands (like break remove or break info) don't clash with operation + argument
- debugger: updated `unwind` crate to 0.4.2, now it must support rcX releases of libunwind
- console: fix expression parser. Now field op, index op and slice op have the same priority and can be combined in any order
- console: now command parser considers spaces when finding subcommands