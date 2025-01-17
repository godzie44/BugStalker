import unittest
from helper import Debugger
import socket
import threading
import time
import signal


def send_tcp_request():
    client = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    client.connect(("localhost", 8080))
    client.send("hello, bs!".encode())
    client.close()


SUPPORTED_TOKIO_V = [
  "1_40",
  "1_41",  
  "1_42",  
  "1_43",  
];


def tokio_binaries():
    binaries = []
    for v in SUPPORTED_TOKIO_V:
        binaries.append(f"tokio_{v}")
    return binaries

class CommandTestCase(unittest.TestCase):           
    def test_runtime_info_1(self):
        for binary in tokio_binaries():
            self.debugger = Debugger(path=f"./examples/tokio_tcp/{binary}/target/debug/{binary}")
            self.debugger.cmd_re('run', r'Listening on: .*:8080')

            thread = threading.Thread(target=send_tcp_request)
            thread.start()
            time.sleep(7)

            self.debugger.debugee_process().send_signal(signal.SIGINT)
            self.debugger.cmd_re(
                'async backtrace',
                r'Thread .* block on:',
                f'async fn {binary}::main',
                'Async worker',
                'Async worker',
                'Async worker'
            )
            self.debugger.cmd_re(
                'async backtrace all',
                r'Thread .* block on:',
                f'async fn {binary}::main',
                'Async worker',
                'Async worker',
                'Async worker',
                f'#0 async fn {binary}::main::{{async_block#0}}',
                'suspended at await point 2',
                '#1 future tokio::sync::oneshot::Receiver<i32>',
                f'#0 async fn {binary}::main::{{async_block#0}}::{{async_block#1}}',
                'suspended at await point 0',
                '#1 sleep future, sleeping',
            )
            # switch to worker thread (hope that thread 2 is a worker)
            self.debugger.cmd('thread switch 2')
            self.debugger.cmd('async task', 'no active task found for current worker')

            # there should be two task with "main" in their names
            self.debugger.cmd('async task .*main.*', 'Task id', 'Task id')

    def test_runtime_info_2(self):
        """Stop async runtime at breakpoint and assert futures state"""
        
        for binary in tokio_binaries():
            self.debugger = Debugger(path=f"./examples/tokio_tcp/{binary}/target/debug/{binary}")
            self.debugger.cmd('break main.rs:54')
            self.debugger.cmd_re('run', r'Listening on: .*:8080')

            thread = threading.Thread(target=send_tcp_request)
            thread.start()
            time.sleep(6)

            self.debugger.cmd_re(
                'async backtrace',
                'Thread .* block on',
                f'#0 async fn {binary}::main',
                'Async worker',
                'Active task',
                f'#0 async fn {binary}::main::{{async_block#0}}'
            )
            self.debugger.cmd(
                'async task',
                f'#0 async fn {binary}::main::{{async_block#0}}',
                'suspended at await point 1',
                '#1 sleep future, sleeping'
            )
            
    def test_step_over(self):
        """Do async step over until future ends"""
        
        for binary in tokio_binaries():
            self.debugger = Debugger(path=f"./examples/tokio_vars/{binary}/target/debug/{binary}")
            self.debugger.cmd('break main.rs:29')
            self.debugger.cmd('run')
            self.debugger.cmd_re('async next', r'Task id: \d', r'30     let _b = inner_1')
            self.debugger.cmd_re('async next', r'Task id: \d', r'32     tokio::time::sleep')
            self.debugger.cmd_re('async next', r'Task id: \d', r'28     let _a')
            self.debugger.cmd_re('async next', r'Task id: \d', r'26 async fn f2')
            self.debugger.cmd_re('async next', r'Task id: \d', r'32     tokio::time::sleep')
            self.debugger.cmd_re('async next', r'Task id: \d', r'33     let _c = inner_1')
            self.debugger.cmd_re('async next', r'Task id: \d', r'34 }')
            self.debugger.cmd_re('async next', r'Task #\d completed, stopped')

    def test_step_out(self):
        """Do async step out"""
        
        for binary in tokio_binaries():
            self.debugger = Debugger(path=f"./examples/tokio_vars/{binary}/target/debug/{binary}")
            self.debugger.cmd('break main.rs:18')
            self.debugger.cmd('break main.rs:28')
            self.debugger.cmd('run', 'Hit breakpoint 1')
            self.debugger.cmd_re('async stepout', r'Task #\d completed, stopped')
            self.debugger.cmd('continue', 'Hit breakpoint 2')
            self.debugger.cmd_re('async stepout', r'Task #\d completed, stopped')

