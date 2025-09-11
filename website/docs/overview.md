---
sidebar_position: 1
---

# Overview

BugStalker is a modern debugger for GNU/Linux x86-64, written in Rust for Rust programs.

import BrowserOnly from '@docusaurus/BrowserOnly';
import AsciinemaPlayer from '@site/src/components/AsciinemaPlayer';

<BrowserOnly>
  {() => <AsciinemaPlayer src="/BugStalker/casts/overview.cast" />}
</BrowserOnly>

## Key Features

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
* **And many more powerful features!**

