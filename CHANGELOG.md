# Changelog

All notable changes to this project will be documented in this file.

# [?.?.?] Unreleased

### Added

- tui: added ability to select tab across both windows
- tui: now left and right windows can expand (and the opposite window,
  accordingly, collapsed)
- ui: new argument (`-t` or `--theme`) for theme switching (affects program data
  and source code output)
- ui: warning if debugee compiled with an unsupported rustc version
- debugger: the index operation is now applicable to hashmaps, hashsets,
  btreemaps and others
- debugger: now containers (hashmaps, hashsets, etc.) can be indexed by literal
  objects for advanced searching
- console: improve index operation, now index accepts literal objects

### Changed

- tui: now current active line (in a source code window and disassemble window)
  glued to the middle of render area instead of the bottom of the screen
- console: now program data (variables and arguments) stylized with syntect

### Fixed

- ui: possible stack overflow when switching between ui types

### Deprecated

### Breaking changes

---

# [0.1.4] April 3 2024

### Changed

- console: history hints now have better highlighting (grey instead of bolt)

### Fixed

- console: now sub commands (like break remove or break info) don't clash with
  operation + argument
- debugger: updated `unwind` crate to 0.4.2, now it must support rcX releases of
  libunwind
- console: fix expression parser. Now field op, index op and slice op have the
  same priority and can be combined in any order
- console: now command parser considers spaces when finding subcommands