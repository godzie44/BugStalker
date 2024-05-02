# BugStalker

> Modern debugger for Linux x86-64. Written in Rust for Rust programs.

![debugger-demo](doc/demo.gif)

---

# Table of Contents

- [BugStalker](#bugstalker)
    * [Supported rustc versions](#supported-rustc-versions)
    * [Features](#features)
    * [Installation](#installation)
    * [Start debugger session](#start-debugger-session)
    * [Help](#help)
    * [Start and restart](#start-and-restart)
    * [Stopping and continuing](#stopping-and-continuing)
    * [Examining the stack](#examining-the-stack)
    * [Examining source files](#examining-source-files)
    * [Examining data](#examining-data)
    * [Other commands](#other-commands)
    * [Tui interface](#tui-interface)
    * [Oracles](#oracles)
- [Contributing](#contributing)

---

## Supported rustc versions

- 1.75
- 1.76
- 1.77

---

## Features

* written in rust for rust language with simplicity as a priority goal
* [breakpoints, steps, signals](#stopping-and-continuing)
* multithreaded application support
* [data query expressions](#examining-data)
* support for a rust type system (collections, smart pointers, thread locals and
  many others), not only for printing but also for interaction
* two ui types: console and [tui](#tui-interface), switch available at any
  moment
* [oracle](#oracles) as an extension mechanism
* builtin [tokio oracle](#oracles) -
  like [tokio_console](https://github.com/tokio-rs/console) but there is no need
  to make changes to the source codes
* and much more!

---

## Installation

First check if the necessary dependencies
(`pkg-config` and `libunwind-dev`) are installed:

For example, ubuntu/debian:

```shell
apt install pkg-config libunwind-dev
```

Now install debugger:

```shell
cargo install bugstalker
```

That's all, `bs` command is available now!

<details>
  <summary>Problem with libunwind?</summary>
If you have any issues with `libunwind`, you can try to install `bs` with
native unwinder 
(currently, I don't recommend this method because libunwind is better :))

```shell
cargo install bugstalker --no-default-features
```

</details>

### Distro Packages

<details>
  <summary>Packaging status</summary>

[![Packaging status](https://repology.org/badge/vertical-allrepos/bugstalker.svg)](https://repology.org/project/bugstalker/versions)

</details>

#### Arch Linux

```shell
pacman -S bugstalker
```

---

## Start debugger session

To start with program from binary file use:

```shell
bs my_cool_program
```

Or with arguments:

```shell
bs my_cool_program -- --arg1 val1 --arg2 val2
```

Or attach to program by its pid:

```shell
bs -p 123
```

## Help

Print `help` for view all available commands.

## Start and restart

[demo](https://www.terminalizer.com/view/2914f76f5890)

- `run` - start or restart a program (alias: `r`)

## Stopping and continuing

The Debugger stops your program when breakpoints are hit,
or after steps commands,
or when the OS signal is coming.
BugStalker always stops the whole program, meaning that all threads are stopped.
Thread witch initiated a stop become a current selected thread.

### Continue execution

[demo](https://terminalizer.com/view/6d5048415891)

- `continue` - resume a stopped program

### Breakpoints

[demo](https://terminalizer.com/view/0a5ee2a05889)

- `break {file}:{line}` - set breakpoint at line (alias: `b {file}:{line}`)
- `break {function name}` - set breakpoint at start of the function (
  alias: `b {function_name}`)
- `break {instruction address}` - set breakpoint at instruction (
  alias: `b {instruction address}`)
- `break remove {number}` - remove breakpoint by its number (
  alias: `b r {number}`)
- `break remove {file}:{line}` - remove breakpoint at line (
  alias: `b r {file}:{line}`)
- `break remove {function name}` - remove breakpoint at start of the function (
  alias: `b r {function name}`)
- `break info` - print all breakpoints

### Steps

[demo](https://terminalizer.com/view/cb4e35a55888)

- `stepi` - step a single instruction
- `step` - step a program until it reaches a different source line (
  alias: `stepinto`)
- `next` - step a program, stepping over subroutine (function) calls (
  alias: `stepover`)
- `finish` - execute a program until selected stack frame returns (
  alias: `stepout`)

### Signals

[demo](https://terminalizer.com/view/4ed500545892)

You can send OS signal to debugee program,
for example, send SIGINT (ctrl+c) to the debugee
program to stop it.

### Change current selected thread

[demo](https://terminalizer.com/view/ad448b5c5893)

- `thread info` - print list of information about threads
- `thread current` - prints current selected thread
- `thread switch {number}` - switch selected thread

## Examining the stack

When your program has stopped,
the first thing you need to know is where it stopped and how it got there.

Each time your program performs a function call,
the information about where in your program the call was made from is saved in a
block of data
called a stack frame.
The frame also contains the arguments of the call and the local variables of the
function
that was called.
All the stack frames are allocated in a region of memory called the call stack.

### Stack frames

The call stack is divided up into contiguous pieces called stack frames.
Each frame is the data associated with one call to one function.
The frame contains the arguments given to the function,
the function's local variables,
and the address at which the function is executed.

### Backtrace

[demo](https://terminalizer.com/view/64f028235898)

- `backtrace` - print backtrace of current stopped thread (alias: `bt`).
  Backtrace contains information about thread
  (number, pid, address of instruction where thread stopped)
  and all frames starting with the currently executing frame (frame zero),
  followed by its caller (frame one), and on up the stack.
- `backtrace all` - print backtraces of all active threads (alias: `bt all`).

### Select a frame

[demo](https://terminalizer.com/view/8ac0ed475896)

Most commands
for examining the stack and other data in your program works
on whichever stack frame is selected at the moment.

- `frame info` - print information about current selected frame.
- `frame switch {num}` - change current selected frame.

## Examining source files

[demo](https://terminalizer.com/view/be63c4b85899)

BugStalker can print parts of your program's source code.
When your program stops,
the debugger spontaneously prints the line where it stopped.
There is `source` commands for print more.

- `source fn` - print current selected function
- `source {num}` - print lines range [current_line-num; current_line+num]
- `source asm` - print assembly representation of current selected function

## Examining data

[demo](https://terminalizer.com/view/418b5da85903)

Of course, you need a way to examine data of your program.

- `var {expression}|locals` command for print local and global variables
- `arg {expression}|all` command for print a function arguments

These commands accept expressions as input or have a special mode
(`var locals` print all local variables, `args all` print all arguments).

### Expression

BugStalker has a special syntax for explore program data.
You can dereference references, get structure fields,
slice arrays or get elements from vectors by its index (and much more!).

Operator available in expressions:

- select variable by its name (ex. `var a`)
- dereference pointers/references/smart pointers (ex. `var *ref_to_a`)
- take a structure field (ex. `var some_struct.some_field`)
- take an element by index from arrays, slices, vectors, hashmaps (
  ex. `var arr[1]`)
- slice arrays, vectors, slices (ex. `var some_vector[1..3]`
  or `var some_vector[1..]`)
- cast constant address to a pointer of a concrete type (
  ex. `(*mut SomeType)0x123AABCD`)
- parentheses for control an operator execution ordering

Write expressions is simple, and you can do it right now!
Some examples:

- `var *some_variable` - dereference and print value of `some_variable`
- `var some_array[0][2..5]` - print 3 elements, starts from index 2 from
  zero element of `some_array`
- `var *some_array[0]` - print dereferenced value of `some_array[0]`
- `var (*some_array)[0]` - print a zero element of `*some_array`
- `var *(*(var1.field1)).field2[1][2]` - print dereferenced value of element at
  index 2 in
  element at index 1 at field `field2` in dereferenced value of field `field1`
  at variable var1 🤡

## Other commands

Of course, the debugger provides many more commands:

- `symbol {name or regex}` - print symbol kind and address
- `memory read {addr}` - read debugged program memory (alias: `mem read`)
- `memory write {addr} {value}` - write into debugged program memory (
  alias: `mem write`)
- `register read {reg_name}` - print value of register by name (x86_64 register
  name in lowercase) (alias: `reg read`)
- `register write {reg_name} {value}` - set new value to register by name (
  alias: `reg write`)
- `register info` - print list of registers with it values (alias: `reg info`)
- `sharedlib info` - show list of shared libraries
- `quit` - exit the BugStalker (alias: `q`)

## Tui interface

[demo](https://terminalizer.com/view/c8de6a1e5901)

One of the most funny BugStalker features is switching between old school
terminal interface and pretty tui at any moment.

- `tui` - switch too terminal ui (in tui use `Esc` for switch back)

## Oracles

[demo console](https://terminalizer.com/view/0ea924865908)

[demo tui](https://terminalizer.com/view/971412185907)

Oracle is a module that expands the capabilities of the debugger.
Oracles can monitor the internal state of a program
to display interesting information.
For example, tokio oracle is able
to provide information about tokio runtime during program debugging without the
need
to change the source code.
You must run the debugger with enabled oracle, for example, for tokio oracle:

```bash
bs --oracle tokio ...
```

Then use `oracle` command for view oracle information:

- `oracle {oracle name} {subcommands}` - run oracle (ex. `oracle tokio`)

Oracles also available in tui.
Currently, there is only one builtin oracle - tokio oracle.

## Contributing

Feel free to suggest changes, ask a question or implement a new feature.
Any contributions are very welcome.
[How to contribute](https://github.com/godzie44/BugStalker/blob/master/CONTRIBUTING.md).
