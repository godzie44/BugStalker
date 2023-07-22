import unittest
import pexpect
import time
import threading
import requests


def make_http_request(event):
    url = "http://localhost:8000"
    payload = "hello from integration test"
    requests.post(url, data=payload)
    event.set()


class PastebinTestCase(unittest.TestCase):
    """Test debugger on application from rocket framework examples"""

    def setUp(self):
        debugger = pexpect.spawn(
            './target/debug/bugstalker ./examples/target/debug/pastebin')
        debugger.expect('BugStalker greets')
        self.debugger = debugger

    def test_step_over_until_response(self):
        """Runs a pastebin application and set breakpoint at http handler. Makes http request, and do `step over` command while http response not returning"""
        self.debugger.sendline('b main.rs:21')

        time.sleep(5)

        self.debugger.sendline('run')
        self.debugger.expect_exact('Configured for debug.')

        event = threading.Event()
        thread = threading.Thread(target=make_http_request, args=(event,))
        thread.start()
        time.sleep(1)

        # check that response returns at this point
        while not event.is_set():
            # send `step over` command otherwise
            self.debugger.sendline('next')
            self.debugger.expect("next")
        time.sleep(0.2)
        thread.join()
        self.debugger.sendline('q')

    def test_continue_until_response(self):
        """Runs a pastebin application and set breakpoint at http handler. Makes http request, do `continue` command and wait until http response not returning"""
        self.debugger.sendline('b main.rs:21')

        time.sleep(5)

        self.debugger.sendline('run')
        self.debugger.expect_exact('Configured for debug.')

        event = threading.Event()
        thread = threading.Thread(target=make_http_request, args=(event,))
        thread.start()
        time.sleep(1)

        self.debugger.expect_exact('21     let id = PasteId::new(ID_LENGTH);')
        self.debugger.sendline('next')
        self.debugger.expect_exact('22     paste')
        self.debugger.sendline('next')
        self.debugger.expect_exact('23         .open(128.kibibytes())')
        self.debugger.sendline('next')
        self.debugger.expect_exact('24         .into_file(id.file_path())')
        self.debugger.sendline('next')
        self.debugger.expect_exact('22     paste')
        self.debugger.sendline('next')
        self.debugger.expect_exact('24         .into_file(id.file_path())')
        self.debugger.sendline('next')
        self.debugger.expect_exact('25         .await?;')

        # check that response returns at this point
        self.debugger.sendline('c')
        thread.join()
        self.debugger.sendline('q')
