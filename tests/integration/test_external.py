import time
import unittest
import pexpect
import psutil
from helper import Debugger


class ExternalProcessTestCase(unittest.TestCase):
    def setUp(self):
        process = pexpect.spawn('./examples/target/debug/sleeper -s 1')
        time.sleep(1)
        self.debugger = Debugger(process=process)

    def test_external_process_connect(self):
        """Assert that debugee can connect to running (or sleeping) process by its pid"""
        self.debugger.cmd('thread current', 'thread id: ' + str(self.debugger.debugee_process().pid))
        self.debugger.cmd('continue', 'exit with code: 0')

    def test_external_process_set_breakpoint(self):
        """Set breakpoint in debugee attached by pid"""
        self.debugger.cmd('break sleeper.rs:24', 'New breakpoint')
        self.debugger.cmd('continue', 'Hit breakpoint 1')
        self.debugger.cmd('continue')

    def test_external_process_view_variables(self):
        """View variables in debugee attached by pid"""
        self.debugger.cmd('break sleeper.rs:24', 'New breakpoint')
        self.debugger.cmd('continue', 'Hit breakpoint 1')
        self.debugger.cmd('var locals', 'sleep_base_sec = u64(1)')

    def test_external_process_restart(self):
        """Restart debugee attached by pid, after restart debugger behaviour should be equivalent to the default one"""
        self.debugger.cmd('break sleeper.rs:24', 'New breakpoint')
        self.debugger.cmd('continue', 'Hit breakpoint 1')
        self.debugger.cmd('run', 'Restart a program?')
        self.debugger.cmd('y', 'Hit breakpoint 1')
        self.debugger.cmd('continue')

    def test_external_process_resume_process(self):
        """Exit from debugger will resume external process"""
        debugee = self.debugger.debugee_process()
        self.assertEqual(debugee.status(), psutil.STATUS_TRACING_STOP)

        self.debugger.cmd('quit')
        time.sleep(0.1)

        debugee = self.debugger.debugee_process()
        self.assertTrue(
            debugee.status() == psutil.STATUS_RUNNING or debugee.status() == psutil.STATUS_SLEEPING,
            "unexpected debugee process status: " + debugee.status(),
        )
