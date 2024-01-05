import unittest
import pexpect
import time
import threading


class OracleTestCase(unittest.TestCase):
    """Test oracles systems"""

    def setUp(self):
        debugger = pexpect.spawn(
            './target/debug/bugstalker --oracle tokio ./examples/target/debug/tokioticker')
        debugger.expect('BugStalker greets')
        self.debugger = debugger

    def test_oracle_unavailable_until_debugee_start(self):
        """Oracles unavailable until program not started"""
        self.debugger.sendline('oracle tokio')
        self.debugger.expect_exact('oracle not found or not ready')

    def test_tokio_oracle(self):
        """Test tokio oracle"""
        self.debugger.sendline('b main.rs:20')
        self.debugger.sendline('b main.rs:24')

        self.debugger.sendline('run')

        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.sendline('oracle tokio')
        self.debugger.expect(r'[1-9]\d? tasks running')

        self.debugger.sendline('continue')

        self.debugger.expect_exact('Hit breakpoint 2')
        self.debugger.sendline('oracle tokio')
        self.debugger.expect_exact('0 tasks running')

        self.debugger.sendline('q')
