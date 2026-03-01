# Examples

A list of applications for debugger testing.

### hello_world

Simple hello world application. Used for test base debugger functions.

### calc

Calculate a sum of input arguments. Used for breakpoint and step testing.

### mt

Multithread application, using it for test thread functions.

### signals

Application with interaction with linux signals.

### vars

Application for test data examination.

### fizzbuzz

Fizzbuzz implementation, artificially overcomplicated.
Used for test related to type polymorphism.

### sleeper

Long live application.
Used to test debugger attaching to external processes.

### recursion

Application with recursion calls.
Used for test debugger behavior with recursive code.

### Pastebin

Example application from [Rocket](https://github.com/SergioBenitez/Rocket) web
framework.

### Todos

Example application from [axum](https://github.com/tokio-rs/axum) web framework.

### Shlib

Example of shared library with C interface and a consumer of this lib.

### Tokioticker

Tick 5 seconds and exit. Useful for tokio oracle testing.

### Panic

Program that just panics.
Initiated by user or system panic (like divide by zero panic).

### Calculations

Program that calculates some values. Useful for watchpoints testing.

### Tokio_tcp

A list of tokio tcp echo-servers with different tokio versions. Useful for testing `async ...` commands.

### Tokio_vars

Tokio application with timers. Useful for testing `async ...` commands.

### Calls

Just some code using for testing a `call` command.

### Dap_set_variable

Application with composite values for DAP `setVariable` testing.

**TODO**
This should be a single application that may compiled into binaries with different library versions. But, for now, looks like this is not possible.

### Dap_exception_details

Nested calls that trigger a SIGSEGV to validate DAP `exceptionInfo` source/stack trace reporting.

### Dap_exception_filters

Application that raises a signal or loops to validate DAP exception breakpoint filters.

### Dap_attach

Application that prints its PID and keeps running for DAP attach testing.

### Dap_source_map

Small application that includes a module under a `./nptl/` path to exercise DAP `source` request
path mapping and normalization fallback logic.

### Dap_disassemble

Application that raises `SIGSTOP` after some arithmetic work to test DAP `disassemble` and
`sourceReference` handling for frames without source information.
