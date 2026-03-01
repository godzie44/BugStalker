# BugStalker

<p align="center">
    <img src="website/static/img/biglogo.png" width="300"></a>
    <br>
</p>

> Modern debugger for Linux x86-64. Written in Rust for Rust programs.

<h4 align="center">
  <a href="https://godzie44.github.io/BugStalker/docs/overview">Documentation</a> |
  <a href="https://godzie44.github.io/BugStalker/">Website</a>
</h4>

---

<div align="center">

<a href="https://github.com/godzie44/BugStalker/releases">
    <img src="https://img.shields.io/github/v/release/godzie44/BugStalker?style=for-the-badge">
</a>

<a href="https://crates.io/crates/bugstalker/">
    <img src="https://img.shields.io/crates/v/bugstalker?style=for-the-badge">
</a>

<a href="https://github.com/godzie44/BugStalker/actions">
    <img src="https://img.shields.io/github/actions/workflow/status/godzie44/BugStalker/ci.yml?style=for-the-badge&label=test">
</a>

<a href="https://docs.rs/bugstalker/">
    <img src="https://img.shields.io/docsrs/bugstalker?style=for-the-badge">
</a>

<img src="https://img.shields.io/crates/l/BugStalker?style=for-the-badge">

</div>

---

<div align="center">

![debugger-demo](website/static/gif/overview.gif)

</div>

---


## Features

* **Rust-native**: Built in Rust specifically for Rust development, with a focus on simplicity
* **Core debugging capabilities:**
  * Breakpoints, step-by-step execution
  * Signal handling
  * Watchpoints
* **Advanced runtime inspection:**
  * Full multithreaded application support
  * Data query expressions
  * Deep Rust type system integration (collections, smart pointers, thread locals, etc.), not only for printing but also for interaction
  * Variable rendering using core::fmt::Debug trait
* **Flexible interfaces:**
  * Switch between console and TUI modes at any time
* **Async Rust support** including Tokio runtime inspection
* **Extensible architecture:**
  * Oracle extension mechanism
  * Built-in tokio oracle (similar to tokio_console but requires no code modifications)
* **DAP (Debug Adapter Protocol) support:**
  * VSCode [extension](https://marketplace.visualstudio.com/items?itemName=BugStalker.bugstalker)
  * Two modes: stdio (embedded) and TCP (remote)
  * See [DAP Documentation](./doc/DAP.md) for details
* **And many more powerful features!**

---

## Installation

See [installation page](https://godzie44.github.io/BugStalker/docs/installation)

## Contributing

Feel free to suggest changes, ask a question or implement a new feature.
Any contributions are very welcome.

[How to contribute](https://github.com/godzie44/BugStalker/blob/master/CONTRIBUTING.md).


## Copyright

© 2026 Derevtsov Konstantin. Distributed under the MIT License.

