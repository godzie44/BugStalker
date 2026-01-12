---
sidebar_position: 1
---

# BS 0.4 released!

After 8 months of development, version 0.4.0 is here - bringing Debug Adapter Protocol (DAP) support and significant performance improvements!

Key highlights:

- DAP Support: Integrate bs directly into VS Code via the [new extension](https://marketplace.visualstudio.com/items?itemName=BugStalker.bugstalker), with support for more DAP-compatible IDEs coming soon.

Special thanks to [@hasali19](https://github.com/hasali19) for contribution.

- Replaced external `libunwind` with a custom unwinder - now the `bs` binary has **no external dependencies**.

Special thanks to [@gvtret](https://github.com/gvtret) for contribution.

- Better performance: optimized for large binaries (e.g., debugging rustc) with reduced memory consumption and faster operation.
- Fixes & improvements: numerous stability enhancements and bug fixes for a smoother debugging experience.

Full changelog since version 0.3.0:

- Added
    - dap: introduce DAP extension for VS Code
    - dap: introduce DAP server
    - debugger: add `GlobalContext`
    - ui: new output for `backtrace` command (with source file and line)
    - debugger: add `--save-history` option
    - debugger: new `DataCast` DQE op  
    - debugger: added support for rustc 1.87 - 1.92 
- Changed
    - build: remove libunwind-specific test target
    - debugger: use string interner
    - debugger: use ecx/ccx/pcx/etc naming for different contexts
    - debugger: parse DIEs on demand rather than upfront to reduce initial memory load
    - debugger: reduce memory consumption for debug information representation
    - debugger: reduce memory consumption for symbol tables
    - update `tui-realm` and `tui-realm-treeview` components
    - add `PopIf::pop_if_single_el`
    - update `chumsky` to a stable version `0.10.1`
    - debugger: now backtrace frames contains a source file and line
- Fixed
    - debugger: panic when vecdeque have infinite capacity (bug in debug info) 
    - build: fail early at compile rather than runtime
    - tui: fix panic when there is a thread with unknown first frame function in backtrace
    - debugger: fix panic when when parse zero-length arrays
    - debugger: enable LTO and codegen-units = 1 for release build
- Deprecated
    - debugger: deprecate `libunwind` support
