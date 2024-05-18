import signal
import time
import unittest
from helper import Debugger


class SignalsTestCase(unittest.TestCase):
    def test_signal_stop_single_thread(self):
        """Send signal to a debugee process"""
        self.debugger = Debugger(path='./examples/target/debug/signals -- single_thread')
        self.debugger.cmd('run')
        time.sleep(1)

        self.debugger.debugee_process().send_signal(signal.SIGUSR1)
        self.debugger.expect_in_output('Signal SIGUSR1 received, debugee stopped')
        self.debugger.cmd('continue', 'got SIGUSR1')

    def test_multi_thread_signal(self):
        """Send signal to a multithread debugee process"""
        self.debugger = Debugger(path='./examples/target/debug/signals -- multi_thread')
        self.debugger.cmd('run')
        time.sleep(1)

        self.debugger.debugee_process().send_signal(signal.SIGUSR1)
        self.debugger.expect_in_output('Signal SIGUSR1 received, debugee stopped')
        self.debugger.cmd('continue', 'threads join')

    def test_multi_thread_multi_signal(self):
        """Send multiple signals to a multithread debugee process"""
        self.debugger = Debugger(path='./examples/target/debug/signals -- multi_thread_multi_signal')
        debugee_process = self.debugger.debugee_process()
        self.debugger.cmd('run')
        time.sleep(1)

        debugee_process.send_signal(signal.SIGUSR1)
        debugee_process.send_signal(signal.SIGUSR2)

        self.debugger.expect_in_output_re(r'Signal SIGUSR[1,2]{1} received, debugee stopped')
        self.debugger.cmd_re('continue', 'Signal SIGUSR[1,2]{1} received, debugee stopped')
        self.debugger.cmd('continue', 'threads join')

    def test_signal_stop_on_continue(self):
        """Send signal to stopped debugee process"""
        self.debugger = Debugger(path='./examples/target/debug/vars')
        self.debugger.cmd('break vars.rs:9', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')

        self.debugger.debugee_process().send_signal(signal.SIGWINCH)
        time.sleep(1)

        self.debugger.cmd('continue', 'Signal SIGWINCH received, debugee stopped')
        self.debugger.cmd('continue', 'Program exit with code: 0')

    def test_signal_stop_on_step(self):
        """Send signal to stop debugee process and do a step"""
        self.debugger = Debugger(path='./examples/target/debug/vars')
        self.debugger.cmd('break vars.rs:9', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')

        self.debugger.debugee_process().send_signal(signal.SIGWINCH)
        time.sleep(1)

        self.debugger.cmd('step', 'Signal SIGWINCH received, debugee stopped')
        self.debugger.cmd('step', '10     let int64 = -2_i64;')

    def test_signal_stop_on_step_over(self):
        """Send signal to stop the debugee process and do a step over"""
        self.debugger = Debugger(path='./examples/target/debug/vars')
        self.debugger.cmd('break vars.rs:9', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')

        self.debugger.debugee_process().send_signal(signal.SIGWINCH)
        time.sleep(1)

        self.debugger.cmd('next', 'Signal SIGWINCH received, debugee stopped')
        self.debugger.cmd('next', '10     let int64 = -2_i64;')

    def test_transparent_signal(self):
        """Send sigint to a running process must return control to debugger"""
        self.debugger = Debugger(path='./examples/target/debug/sleeper -- -s 5')
        self.debugger.cmd('run')

        time.sleep(3)

        self.debugger.control('c')
        self.debugger.expect_in_output('Signal SIGINT received, debugee stopped')
        self.debugger.cmd('bt', 'sleeper::main')
