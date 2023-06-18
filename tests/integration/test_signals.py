import re
import signal
import time
import unittest
import pexpect
import psutil


class SignalsTestCase(unittest.TestCase):
    def test_signal_stop_single_thread(self):
        """Send signal to debugee process"""
        self.debugger = pexpect.spawn(
            './target/debug/bugstalker ./target/debug/signals -- single_thread')
        self.debugger.expect('BugStalker greets')

        debugger_process = psutil.Process(self.debugger.pid)
        debugee_process = debugger_process.children(recursive=False)[0]

        self.debugger.sendline('run')
        time.sleep(1)

        debugee_process.send_signal(signal.SIGUSR1)
        self.debugger.expect_exact('Receive signal SIGUSR1, debugee stopped')

        self.debugger.sendline('continue')
        self.debugger.expect_exact('got SIGUSR1')

    def test_multi_thread_signal(self):
        """Send signal to multithread debugee process"""
        self.debugger = pexpect.spawn(
            './target/debug/bugstalker ./target/debug/signals -- multi_thread')
        self.debugger.expect('BugStalker greets')

        debugger_process = psutil.Process(self.debugger.pid)
        debugee_process = debugger_process.children(recursive=False)[0]

        self.debugger.sendline('run')
        time.sleep(1)

        debugee_process.send_signal(signal.SIGUSR1)
        self.debugger.expect_exact('Receive signal SIGUSR1, debugee stopped')

        self.debugger.sendline('continue')
        self.debugger.expect_exact('threads join')

    def test_multi_thread_multi_signal(self):
        """Send multiple signals to multithread debugee process"""
        self.debugger = pexpect.spawn(
            './target/debug/bugstalker ./target/debug/signals -- multi_thread_multi_signal')
        self.debugger.expect('BugStalker greets')

        debugger_process = psutil.Process(self.debugger.pid)
        debugee_process = debugger_process.children(recursive=False)[0]

        self.debugger.sendline('run')
        time.sleep(1)

        debugee_process.send_signal(signal.SIGUSR1)
        debugee_process.send_signal(signal.SIGUSR2)

        self.debugger.expect_exact('Receive signal SIGUSR1, debugee stopped')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('Receive signal SIGUSR2, debugee stopped')
        self.debugger.sendline('continue')

        self.debugger.expect_exact('threads join')
