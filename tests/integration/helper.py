import pexpect
import re

import psutil


class Debugger:
    def __init__(self, path=None, process=None, oracles=None):
        self._external_debugee_process = None
        base = './target/release/bs -t none'
        if oracles is None:
            oracles = []
        for oracle in oracles:
            base = f'{base} --oracle {oracle}'
        if path:
            self._process = pexpect.spawn(f'{base} {path}')
        if process:
            self._external_debugee_process = process
            pid = self._external_debugee_process.pid
            self._process = pexpect.spawn(f'{base} -p {pid}')
        self._process.expect_exact('BugStalker greets')

    def cmd(self, cmd, *should_see):
        self._process.sendline(cmd)
        for output in should_see:
            self._process.expect_exact(output)

    def cmd_re(self, cmd, *should_see_re):
        self._process.sendline(cmd)
        for regex in should_see_re:
            self._process.expect(regex)

    def print(self, s, *should_see):
        self._process.send(s)
        for output in should_see:
            self._process.expect_exact(output)

    def control(self, char):
        self._process.sendcontrol(char)

    def search_in_output(self, pattern, line_cnt=10):
        for x in range(line_cnt):
            line = self._process.readline().decode("utf-8")
            result = re.search(pattern, line)
            if result:
                return result.group(1)

    def debugger_process(self):
        return psutil.Process(self._process.pid)

    def debugee_process(self):
        if self._external_debugee_process is not None:
            return psutil.Process(self._external_debugee_process.pid)
        else:
            debugger_process = self.debugger_process()
            return debugger_process.children(recursive=False)[0]

    def is_alive(self):
        return self._process.isalive()

    def expect_in_output(self, text, timeout=-1):
        self._process.expect_exact(text, timeout)
        
    def expect_in_output_re(self, regex, timeout=-1):
        self._process.expect(regex, timeout)
