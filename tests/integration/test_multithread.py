import time
import unittest
import pexpect
from helper import Debugger


class MultithreadTestCase(unittest.TestCase):
    def setUp(self):
        self.debugger = Debugger(path='./examples/target/debug/mt')

    def test_multithreaded_app_running(self):
        """Multithread debugee executing"""
        self.debugger.cmd(
            'run',
            'thread 1 spawn',
            'thread 2 spawn',
            'sum2: 199990000',
            'sum1: 49995000',
            'total 249985000',
        )

    def test_multithreaded_breakpoints(self):
        """Multithread debugee breakpoints"""
        # set breakpoint at program start
        self.debugger.cmd('break mt.rs:6', 'New breakpoint')
        # set breakpoints at thread 1 code
        self.debugger.cmd('break mt.rs:24', 'New breakpoint')
        # set breakpoint at thread 2 code
        self.debugger.cmd('break mt.rs:36', 'New breakpoint')
        # set breakpoint at program ends
        self.debugger.cmd('break mt.rs:14', 'New breakpoint')

        self.debugger.cmd('run', 'Hit breakpoint 1 at', '6     let jh1 = thread::spawn(sum1);')
        self.debugger.cmd(
            'continue',
            'thread 1 spawn',
            'thread 2 spawn',
            'Hit breakpoint 3 at',
            '36     let mut sum2 = 0;',
        )
        self.debugger.cmd('continue', 'Hit breakpoint 2 at', '24     let mut sum = 0;')
        self.debugger.cmd('continue', 'Hit breakpoint 4 at', '14     println!("total {}", sum1 + sum2);')
        self.debugger.cmd('continue', 'total 249985000')

    def test_multithreaded_backtrace(self):
        """Backtrace command for multithread debugee"""
        self.debugger.cmd('break mt.rs:24', 'New breakpoint')
        self.debugger.cmd('run', 'thread 1 spawn', 'Hit breakpoint 1 at', '24     let mut sum = 0;')
        self.debugger.cmd('backtrace', 'mt::sum1', 'new::thread_start')

    def test_multithreaded_trace(self):
        """Trace command for multithread debugee"""
        self.debugger.cmd('break mt.rs:36', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1 at', '36     let mut sum2 = 0;')
        self.debugger.cmd(
            'backtrace all',
            'thread',
            'mt::main',
            'thread',
            'clock_nanosleep',
            'thread',
            'mt::sum2',
        )

    def test_multithreaded_quit(self):
        """Quit command for multithread debugee"""
        self.debugger.cmd('break mt.rs:36', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1 at', '36     let mut sum2 = 0;')
        self.debugger.cmd('quit')
        time.sleep(2)
        self.assertFalse(self.debugger.is_alive())

    def test_thread_info(self):
        """Thread info/current command for multithread debugee"""
        self.debugger.cmd('break mt.rs:40', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1 at')
        self.debugger.cmd('thread info', '#1 thread id', '#2 thread id', '#3 thread id')
        self.debugger.cmd('thread current', '#3 thread id')

    def test_thread_switch(self):
        """Trace switch command for multithread debugee"""
        self.debugger.cmd('break mt.rs:40', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1 at')
        self.debugger.cmd('thread current', '#3 thread id')
        # switch to another thread
        self.debugger.cmd('thread switch 2', 'Thread #2 brought into focus')
        self.debugger.cmd('thread current', '#2 thread id')

        # try to step into a new in-focus thread, if there is no debug info for shared libs
        # in a system (libc in this case), we must do a single step, if debug info exists
        # two steps is needed
        try:
            self.debugger.cmd('step', '/linux/nanosleep.c')
        except pexpect.ExceptionPexpect:
            self.debugger.cmd('step', '/linux/nanosleep.c')

        self.debugger.cmd('step')
        time.sleep(0.2)
        self.debugger.cmd('step')
        time.sleep(0.2)
        self.debugger.cmd('step')
        time.sleep(0.2)

        self.debugger.expect_in_output('24     let mut sum = 0;')
        self.debugger.cmd('step')
        time.sleep(0.2)

        # try to view variables from a new in-focus thread
        self.debugger.cmd('var locals', 'sum = i32(0)')

    def test_thread_switch_frame_switch(self):
        """Trace switch and frame switch command for multithread debugee"""
        self.debugger.cmd('break mt.rs:40', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1 at')
        self.debugger.cmd('thread current', '#3 thread id')
        # switch to another thread
        self.debugger.cmd('thread switch 2', 'Thread #2 brought into focus')
        self.debugger.cmd('thread current', '#2 thread id')
        self.debugger.cmd('frame switch 3', 'switch to #3')
        self.debugger.cmd('var locals', 'sum3_jh = JoinHandle<i32> {')
