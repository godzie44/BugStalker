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
        self.debugger.expect_exact('Signal SIGUSR1 received, debugee stopped')

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
        self.debugger.expect_exact('Signal SIGUSR1 received, debugee stopped')

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

        self.debugger.expect('Signal SIGUSR[1,2]{1} received, debugee stopped')
        self.debugger.sendline('continue')
        self.debugger.expect('Signal SIGUSR[1,2]{1} received, debugee stopped')
        self.debugger.sendline('continue')

        self.debugger.expect_exact('threads join')

    def test_signal_stop_on_continue(self):
        """Send signal to stopped debugee process"""
        self.debugger = pexpect.spawn(
            './target/debug/bugstalker ./target/debug/vars')
        self.debugger.expect('BugStalker greets')

        debugger_process = psutil.Process(self.debugger.pid)
        debugee_process = debugger_process.children(recursive=False)[0]

        self.debugger.sendline('break vars.rs:9')
        self.debugger.expect_exact('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        debugee_process.send_signal(signal.SIGWINCH)
        time.sleep(1)

        self.debugger.sendline('continue')
        self.debugger.expect_exact('Signal SIGWINCH received, debugee stopped')

        self.debugger.sendline('continue')
        self.debugger.expect_exact('Program exit with code: 0')

    def test_signal_stop_on_step(self):
        """Send signal to stopped debugee process and do a step"""
        self.debugger = pexpect.spawn(
            './target/debug/bugstalker ./target/debug/vars')
        self.debugger.expect('BugStalker greets')

        debugger_process = psutil.Process(self.debugger.pid)
        debugee_process = debugger_process.children(recursive=False)[0]

        self.debugger.sendline('break vars.rs:9')
        self.debugger.expect_exact('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        debugee_process.send_signal(signal.SIGWINCH)
        time.sleep(1)

        self.debugger.sendline('step')
        self.debugger.expect_exact('Signal SIGWINCH received, debugee stopped')

        self.debugger.sendline('step')
        self.debugger.expect_exact('10     let int64 = -2_i64;')

    def test_signal_stop_on_step_over(self):
        """Send signal to stopped debugee process and do a step over"""
        self.debugger = pexpect.spawn(
            './target/debug/bugstalker ./target/debug/vars')
        self.debugger.expect('BugStalker greets')

        debugger_process = psutil.Process(self.debugger.pid)
        debugee_process = debugger_process.children(recursive=False)[0]

        self.debugger.sendline('break vars.rs:9')
        self.debugger.expect_exact('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        debugee_process.send_signal(signal.SIGWINCH)
        time.sleep(1)

        self.debugger.sendline('next')
        self.debugger.expect_exact('Signal SIGWINCH received, debugee stopped')

        self.debugger.sendline('next')
        self.debugger.expect_exact('10     let int64 = -2_i64;')

