import unittest
import pexpect


class MultithreadTestCase(unittest.TestCase):
    def setUp(self):
        debugger = pexpect.spawn('./target/debug/bugstalker ./target/debug/mt')
        debugger.expect('BugStalker greets')
        self.debugger = debugger

    def test_multithreaded_app_running(self):
        """Multithread debugee executing"""
        self.debugger.sendline('run')
        self.debugger.expect('thread 1 spawn')
        self.debugger.expect('thread 2 spawn')
        self.debugger.expect('sum2: 199990000')
        self.debugger.expect('sum1: 49995000')
        self.debugger.expect('total 249985000')

    def test_multithreaded_breakpoints(self):
        """Multithread debugee breakpoints"""
        # set breakpoint at program start
        self.debugger.sendline('break mt.rs:6')
        self.debugger.expect('New breakpoint')
        # set breakpoints at thread 1 code
        self.debugger.sendline('break mt.rs:22')
        self.debugger.expect('New breakpoint')
        # set breakpoint at thread 2 code
        self.debugger.sendline('break mt.rs:34')
        self.debugger.expect('New breakpoint')
        # set breakpoint at program ends
        self.debugger.sendline('break mt.rs:14')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint 1 at')
        self.debugger.expect_exact('6     let jh1 = thread::spawn(sum1);')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('thread 1 spawn')
        self.debugger.expect_exact('thread 2 spawn')
        self.debugger.expect_exact('sum3 (unused): 45')
        self.debugger.expect('Hit breakpoint 3 at')
        self.debugger.expect_exact('34     let mut sum2 = 0;')
        self.debugger.sendline('continue')
        self.debugger.expect('Hit breakpoint 2 at')
        self.debugger.expect_exact('22     let mut sum = 0;')
        self.debugger.sendline('continue')
        self.debugger.expect('Hit breakpoint 4 at')
        self.debugger.expect_exact('14     println!("total {}", sum1 + sum2);')
        self.debugger.sendline('continue')
        self.debugger.expect('total 249985000')

    def test_multithreaded_backtrace(self):
        """Backtrace command for multithread debugee"""
        self.debugger.sendline('break mt.rs:22')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('thread 1 spawn')
        self.debugger.expect('Hit breakpoint 1 at')
        self.debugger.expect('22     let mut sum = 0;')
        self.debugger.sendline('backtrace')
        self.debugger.expect('mt::sum1')
        self.debugger.expect('std::sys::unix::thread::Thread::new::thread_start')

    def test_multithreaded_trace(self):
        """Trace command for multithread debugee"""
        self.debugger.sendline('break mt.rs:34')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint 1 at')
        self.debugger.expect('34     let mut sum2 = 0;')
        self.debugger.sendline('backtrace all')
        self.debugger.expect('thread')
        self.debugger.expect('mt::main')
        self.debugger.expect('thread')
        self.debugger.expect('std::thread::sleep')
        self.debugger.expect('thread')
        self.debugger.expect('mt::sum2')
