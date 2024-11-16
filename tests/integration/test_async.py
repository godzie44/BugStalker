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
];

def tokio_binaries():
    binaries = []
    for v in SUPPORTED_TOKIO_V:
        binaries.append(f"tokio_tcp_{v}")
    return binaries

class CommandTestCase(unittest.TestCase):           
    def test_runtime_info_1(self):
        for binary in tokio_binaries():
            self.debugger = Debugger(path=f"./examples/target/debug/{binary}")
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
            self.debugger = Debugger(path=f"./examples/target/debug/{binary}")
            self.debugger.cmd('break main.rs:59')
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
