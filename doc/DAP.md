# BugStalker Debug Adapter Protocol (DAP) Documentation

BugStalker provides full support for the [Debug Adapter Protocol](https://microsoft.github.io/debug-adapter-protocol/), allowing integration with various IDEs and debugging tools.

## Overview

BugStalker can operate in two DAP modes:

1. **Stdio Mode** (`--dap-local`) - For embedded debugging in IDEs like VSCode
2. **TCP Mode** (`--dap-remote`) - For remote debugging or standalone server mode

## Quick Start

### VSCode Integration (Stdio Mode)

Install the official [BugStalker VSCode extension](https://marketplace.visualstudio.com/items?itemName=BugStalker.bugstalker). It uses stdio mode automatically.

### Command Line Debugging (TCP Mode)

Start BugStalker as a DAP server:

```bash
# Listen on localhost:4711
bs --dap-remote 127.0.0.1:4711 ./my_program

# Listen on all interfaces
bs --dap-remote 0.0.0.0:4711 ./my_program

# Single client mode (exit after first session)
bs --dap-remote 127.0.0.1:4711 --dap-oneshot ./my_program
```

## CLI Flags

### DAP Stdio Mode

```bash
# Enable DAP in stdio mode (for IDE integration)
bs --dap-local ./my_program

# With logging
bs --dap-local --dap-log-file debug.log --dap-trace ./my_program
```

**Flags:**
- `--dap-local` - Enable DAP in stdio mode
- `--dap-log-file <PATH>` - Log DAP traffic to file (optional)
- `--dap-trace` - Trace DAP messages to log file (requires `--dap-log-file`)

### DAP TCP Mode

```bash
# Basic TCP server
bs --dap-remote 127.0.0.1:4711 ./my_program

# Single session mode
bs --dap-remote 127.0.0.1:4711 --dap-oneshot ./my_program

# With diagnostics
bs --dap-remote 127.0.0.1:4711 --dap-log-file server.log --dap-trace
```

**Flags:**
- `--dap-remote <ADDR>` - Enable DAP TCP server on address:port
- `--dap-oneshot` - Exit after first debug session
- `--dap-log-file <PATH>` - Log adapter diagnostics (for debugging the adapter itself)
- `--dap-trace` - Trace all DAP protocol messages

### Combining with Other Options

```bash
# TCP DAP with debugee arguments
bs --dap-remote 127.0.0.1:4711 ./my_program arg1 arg2

# TCP DAP with working directory
bs --dap-remote 127.0.0.1:4711 --cwd /path/to/workdir ./my_program

# TCP DAP with Rust stdlib path
bs --dap-remote 127.0.0.1:4711 --std-lib-path /path/to/lib ./my_program

# TCP DAP with oracle configuration
bs --dap-remote 127.0.0.1:4711 --oracle tokio ./my_program
```

## Architecture

### Transport Layer

BugStalker uses an abstraction layer for DAP transports:

```
DAP Session (DebugSession)
    ↓
DapTransport trait
    ↓
    ├─ StdioTransport (stdin/stdout)
    └─ TcpTransport (TCP sockets)
```

Both transports implement the same DAP framing protocol:
- Message header with `Content-Length: N\r\n\r\n`
- JSON message body

### Session Management

**Stdio Mode:**
- Single session per program run
- IDE manages lifecycle
- Ideal for integrated debugging

**TCP Mode:**
- Can accept multiple clients sequentially (default)
- Or single client with `--dap-oneshot`
- Each client gets a new debug session for the same program
- Useful for:
  - Remote debugging
  - Custom debugging tools
  - Debugging from multiple machines

## Protocol Support

### Implemented Requests

**Session Control:**
- `initialize` - Initialize debugger
- `launch` - Launch debugee with arguments
- `attach` - Attach to running process
- `disconnect` - Disconnect from debugee
- `terminate` - Terminate debugee

**Execution:**
- `configurationDone` - Resume after configuration
- `continue` - Continue execution
- `next` - Step over
- `stepIn` - Step into
- `stepOut` - Step out
- `pause` - Pause execution

**Breakpoints:**
- `setBreakpoints` - Set line breakpoints
- `breakpointLocations` - Get valid breakpoint locations
- `setDataBreakpoints` - Set watchpoints
- `setInstructionBreakpoints` - Set instruction breakpoints
- `setFunctionBreakpoints` - Set function breakpoints

**Stack Inspection:**
- `stackTrace` - Get call stack
- `threads` - Get thread list
- `modules` - Get loaded modules
- `scopes` - Get scope variables
- `variables` - Get variable details
- `source` - Get source code/disassembly

**Debugging:**
- `evaluate` - Evaluate expressions
- `setVariable` - Modify variables
- `readMemory` - Read process memory
- `disassemble` - Disassemble code
- `goto` - Jump to address
- `gotoTargets` - Get possible jump targets

### Events Supported

- `initialized` - Debugger initialized
- `stopped` - Execution stopped (breakpoint, signal, etc.)
- `continued` - Execution resumed
- `threads` - Threads changed
- `terminated` - Debugee terminated
- `exited` - Debugee exited
- `breakpoint` - Breakpoint state changed
- `module` - Module loaded/unloaded
- `output` - Output from debugee or adapter
- `process` - Process information
- `progressStart`/`progressUpdate`/`progressEnd` - Long operations

## Capabilities

BugStalker reports the following DAP capabilities:

```json
{
  "supportsConfigurationDoneRequest": true,
  "supportsSetVariable": true,
  "supportsBreakpointLocationsRequest": true,
  "supportsDataBreakpoints": true,
  "supportsInstructionBreakpoints": true,
  "supportsFunctionBreakpoints": true,
  "supportsGotoTargetsRequest": true,
  "supportsRestartFrame": true,
  "supportsEvaluateForHovers": true,
  "supportsStepBack": false,
  "supportsReadMemoryRequest": true,
  "supportsDisassembleRequest": true,
  "supportsTerminateRequest": true,
  "supportsDelayedStackTraceRequest": false,
  "supportsLoadedSourcesRequest": true,
  "supportsLogPoints": true,
  "supportsTerminateThreadsRequest": true,
  "supportsExceptionFilterOptions": true,
  "supportsExceptionInfoRequest": true,
  "supportsModulesRequest": true,
  "supportsCancelRequest": true
}
```

## Examples

### Example 1: Debug in VSCode

1. Install [BugStalker extension](https://marketplace.visualstudio.com/items?itemName=BugStalker.bugstalker)
2. Create `.vscode/launch.json`:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "name": "BugStalker (Launch)",
      "type": "bugstalker",
      "request": "launch",
      "program": "${workspaceFolder}/target/debug/my_app",
      "args": [],
      "cwd": "${workspaceFolder}",
      "stopOnEntry": false
    },
    {
      "name": "BugStalker (Attach)",
      "type": "bugstalker",
      "request": "attach",
      "pid": "${command:pickProcess}"
    }
  ]
}
```

3. Open your Rust file and set breakpoints
4. Press F5 or select configuration in Run menu

### Example 2: Remote Debugging

**Server side** (machine A):
```bash
# Start BugStalker TCP server
bs --dap-remote 0.0.0.0:4711 ./my_program
```

**Client side** (machine B):
Connect from IDE or custom client to `machine-a:4711`

### Example 3: Custom Debugging Tool

```python
import socket
import json

# Connect to DAP server
sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.connect(('localhost', 4711))

def send_request(command, arguments):
    msg = {
        "seq": 1,
        "type": "request",
        "command": command,
        "arguments": arguments
    }
    payload = json.dumps(msg).encode()
    header = f"Content-Length: {len(payload)}\r\n\r\n".encode()
    sock.send(header + payload)

def read_response():
    # Read Content-Length
    header = b''
    while not header.endswith(b'\r\n\r\n'):
        header += sock.recv(1)
    
    # Parse content length
    lines = header.decode().strip().split('\r\n')
    length = int(lines[0].split(': ')[1])
    
    # Read message
    message = sock.recv(length)
    return json.loads(message)

# Initialize
send_request("initialize", {
    "clientID": "custom-tool",
    "clientName": "My Custom Tool"
})
response = read_response()
print(f"Capabilities: {response['body']}")

sock.close()
```

## Troubleshooting

### "Port already in use"

Another instance of BugStalker is listening on the same port. Either:
- Kill the existing process: `killall bs`
- Use a different port: `bs --dap-remote 127.0.0.1:4712 ./program`

### IDE not connecting

1. Verify BugStalker is listening: `netstat -ln | grep 4711`
2. Check firewall rules
3. If using remote machine, verify connectivity: `nc -zv remote-host 4711`

### Slow stepping/breakpoints

This might happen with:
- Large binaries with lots of debug info
- Network latency in remote mode

**Solutions:**
- Reduce debug info: use `debug = 1` in `Cargo.toml`
- Use local TCP socket instead of network
- Report performance issues

### DAP messages not appearing in logs

1. Use `--dap-log-file <PATH>` to specify log file
2. Use `--dap-trace` flag to enable message tracing
3. Ensure the path is writable: `touch /tmp/test.log` to verify

## Testing

BugStalker includes comprehensive DAP tests:

```bash
# Run all DAP tests
cargo test --test dap

# Run only TCP tests
cargo test --test dap dap_integration

# Run only stdio tests  
cargo test --test dap dap_stdio

# Run with output
cargo test --test dap -- --nocapture
```

## IDE Support Matrix

| IDE | Type | Extension | Status |
|-----|------|-----------|--------|
| VSCode | Stdio | [Official](https://marketplace.visualstudio.com/items?itemName=BugStalker.bugstalker) | ✅ Fully supported |
| Neovim | TCP | Community | 🟡 Via DAP plugins |
| Emacs | TCP | dap-mode | 🟡 Via DAP plugins |
| JetBrains IDEs | TCP | Custom | 🟡 Manual setup |

## Performance Considerations

### Stdio Mode
- **Pros:** Low overhead, integrated experience
- **Cons:** Blocked by single session
- **Best for:** IDE integration

### TCP Mode
- **Pros:** Multiple clients, remote debugging, high scalability
- **Cons:** Network latency
- **Best for:** Remote debugging, custom tools

## Configuration Examples

### Development

```bash
# Standard debugging with full logging
bs --dap-remote 127.0.0.1:4711 \
   --dap-log-file /tmp/bs-dap.log \
   --dap-trace \
   ./target/debug/myapp
```

### Production Debugging

```bash
# Minimal overhead, single session
bs --dap-remote 0.0.0.0:4711 \
   --dap-oneshot \
   ./target/release/myapp
```

### CI/CD Integration

```bash
# VSCode debugging in CI environment
bs --dap-local \
   --dap-log-file /tmp/test-dap.log \
   ./target/debug/integration_test
```

## Related Documentation

- [Debug Adapter Protocol Specification](https://microsoft.github.io/debug-adapter-protocol/)
- [VSCode Extension Guide](./VSCode-Extension.md) (if exists)
- [Main README](../README.md)
- [Installation Guide](./installation.md)
