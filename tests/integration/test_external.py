import time
import unittest
import pexpect
import psutil


class ExternalProcessTestCase(unittest.TestCase):
    def setUp(self):
        self.process = pexpect.spawn('./examples/target/debug/sleeper -s 1')
        self.process_pid = self.process.pid
        time.sleep(1)
        self.debugger = pexpect.spawn('./target/debug/bs -t none -p ' + str(self.process_pid))

    def test_external_process_connect(self):
        """Assert that debugee can connect to running (or sleeping) process by its pid"""
        self.debugger.expect('BugStalker greets')
        self.debugger.sendline('thread current')
        self.debugger.expect('thread id: ' + str(self.process_pid))
        self.debugger.sendline('continue')
        self.debugger.expect('exit with code: 0')

    def test_external_process_set_breakpoint(self):
        """Set breakpoint in debugee attached by pid"""
        self.debugger.expect('BugStalker greets')
        self.debugger.sendline('break sleeper.rs:24')
        self.debugger.expect('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.sendline('continue')

    def test_external_process_view_variables(self):
        """View variables in debugee attached by pid"""
        self.debugger.expect('BugStalker greets')
        self.debugger.sendline('break sleeper.rs:24')
        self.debugger.expect('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.sendline('var locals')
        self.debugger.expect_exact('sleep_base_sec = u64(1)')

    def test_external_process_restart(self):
        """Restart debugee attached by pid, after restart debugger behaviour should be equivalent to the default one"""
        self.debugger.expect('BugStalker greets')
        self.debugger.sendline('break sleeper.rs:24')
        self.debugger.expect('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.sendline('run')
        self.debugger.expect('Restart a program?')
        self.debugger.sendline('y')
        self.debugger.expect_exact('Hit breakpoint 2')
        self.debugger.sendline('continue')

    def test_external_process_resume_process(self):
        """Exit from debugger will resume external process"""
        self.debugger.expect('BugStalker greets')

        debugee = psutil.Process(self.process_pid)
        self.assertEqual(debugee.status(), psutil.STATUS_TRACING_STOP)

        self.debugger.sendline('quit')

        time.sleep(0.1)

        debugee = psutil.Process(self.process_pid)
        self.assertTrue(
            debugee.status() == psutil.STATUS_RUNNING or debugee.status() == psutil.STATUS_SLEEPING,
            "unexpected debugee process status: " + debugee.status(),
        )

