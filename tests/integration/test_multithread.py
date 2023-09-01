import time
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
        self.debugger.sendline('break mt.rs:24')
        self.debugger.expect('New breakpoint')
        # set breakpoint at thread 2 code
        self.debugger.sendline('break mt.rs:36')
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
        self.debugger.expect('Hit breakpoint 3 at')
        self.debugger.expect_exact('36     let mut sum2 = 0;')
        self.debugger.sendline('continue')
        self.debugger.expect('Hit breakpoint 2 at')
        self.debugger.expect_exact('24     let mut sum = 0;')
        self.debugger.sendline('continue')
        self.debugger.expect('Hit breakpoint 4 at')
        self.debugger.expect_exact('14     println!("total {}", sum1 + sum2);')
        self.debugger.sendline('continue')
        self.debugger.expect('total 249985000')

    def test_multithreaded_backtrace(self):
        """Backtrace command for multithread debugee"""
        self.debugger.sendline('break mt.rs:24')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('thread 1 spawn')
        self.debugger.expect('Hit breakpoint 1 at')
        self.debugger.expect('24     let mut sum = 0;')
        self.debugger.sendline('backtrace')
        self.debugger.expect('mt::sum1')
        self.debugger.expect('new::thread_start')

    def test_multithreaded_trace(self):
        """Trace command for multithread debugee"""
        self.debugger.sendline('break mt.rs:36')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint 1 at')
        self.debugger.expect('36     let mut sum2 = 0;')
        self.debugger.sendline('backtrace all')
        self.debugger.expect('thread')
        self.debugger.expect('mt::main')
        self.debugger.expect('thread')
        self.debugger.expect('clock_nanosleep')
        self.debugger.expect('thread')
        self.debugger.expect('mt::sum2')

    def test_multithreaded_quit(self):
        """Quit command for multithread debugee"""
        self.debugger.sendline('break mt.rs:36')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint 1 at')
        self.debugger.expect('36     let mut sum2 = 0;')
        self.debugger.sendline('quit')
        time.sleep(2)
        self.assertFalse(self.debugger.isalive())

    def test_thread_info(self):
        """Thread info/current command for multithread debugee"""
        self.debugger.sendline('break mt.rs:40')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint 1 at')

        self.debugger.sendline('thread info')
        self.debugger.expect_exact('#1 thread id')
        self.debugger.expect_exact('#2 thread id')
        self.debugger.expect_exact('#3 thread id')

        self.debugger.sendline('thread current')
        self.debugger.expect_exact('#3 thread id')

    def test_thread_switch(self):
        """Trace switch command for multithread debugee"""
        self.debugger.sendline('break mt.rs:40')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint 1 at')

        self.debugger.sendline('thread current')
        self.debugger.expect_exact('#3 thread id')

        # switch to another thread
        self.debugger.sendline('thread switch 2')
        self.debugger.expect_exact('Thread #2 brought into focus')

        self.debugger.sendline('thread current')
        self.debugger.expect_exact('#2 thread id')

        # try to step in new in focus thread, if there is no debug info for shared libs
        # in system (libc in this case) we must do a single step, if debug info exists
        # two steps is needed
        try:
            self.debugger.sendline('step')
            self.debugger.expect_exact('/sys/unix/thread.rs')
        except pexpect.ExceptionPexpect:
            self.debugger.sendline('step')
            self.debugger.expect_exact('/sys/unix/thread.rs')

        self.debugger.sendline('step')
        self.debugger.sendline('step')
        self.debugger.expect_exact('24     let mut sum = 0;')
        self.debugger.sendline('step')

        # try to view variables from a new in focus thread
        self.debugger.sendline('var locals')
        self.debugger.expect_exact('sum = i32(0)')

    def test_thread_switch_frame_switch(self):
        """Trace switch and frame switch command for multithread debugee"""
        self.debugger.sendline('break mt.rs:40')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint 1 at')

        self.debugger.sendline('thread current')
        self.debugger.expect_exact('#3 thread id')

        # switch to another thread
        self.debugger.sendline('thread switch 2')
        self.debugger.expect_exact('Thread #2 brought into focus')

        self.debugger.sendline('thread current')
        self.debugger.expect_exact('#2 thread id')

        self.debugger.sendline('frame switch 3')
        self.debugger.expect_exact('switch to #3')
        self.debugger.sendline('var locals')
        self.debugger.expect_exact('sum3_jh = JoinHandle<i32> {')
