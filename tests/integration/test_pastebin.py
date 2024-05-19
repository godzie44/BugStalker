import unittest
import time
import threading
import requests
from helper import Debugger


def make_http_request(event):
    url = "http://localhost:8000"
    payload = "hello from integration test"
    requests.post(url, data=payload)
    event.set()


class PastebinTestCase(unittest.TestCase):
    """Test a debugger on application from rocket framework examples"""

    def setUp(self):
        self.debugger = Debugger(path='./examples/target/debug/pastebin')

    def test_step_over_until_response(self):
        """Runs a pastebin application and set breakpoint at http handler. Makes http request, and do `step over` command while http response not returning"""
        self.debugger.cmd('b main.rs:21', 'New breakpoint')

        time.sleep(3)

        self.debugger.cmd_re('run', 'Configured for debug.')

        event = threading.Event()
        thread = threading.Thread(target=make_http_request, args=(event,))
        thread.start()
        time.sleep(3)

        # check that response returns at this point
        while not event.wait(0.2):
            # send `step over` command otherwise
            self.debugger.cmd('next', 'next')
            time.sleep(0.1)
        time.sleep(0.2)
        thread.join()
        self.debugger.control('c')
        self.debugger.cmd('q')

    def test_continue_until_response(self):
        """Runs a pastebin application and set breakpoint at http handler. Makes http request, do `continue` command and wait until http response not returning"""
        self.debugger.cmd('b main.rs:21', 'New breakpoint')

        time.sleep(3)

        self.debugger.cmd('run', 'Configured for debug.')

        event = threading.Event()
        thread = threading.Thread(target=make_http_request, args=(event,))
        thread.start()
        time.sleep(2)

        self.debugger.expect_in_output('21     let id = PasteId::new(ID_LENGTH);')
        self.debugger.cmd('next', '22     paste')
        self.debugger.cmd('next', '23         .open(128.kibibytes())')
        self.debugger.cmd('next', '24         .into_file(id.file_path())')
        self.debugger.cmd('next', '22     paste')
        self.debugger.cmd('next', '24         .into_file(id.file_path())')
        self.debugger.cmd('next', '25         .await?;')

        # check that response returns at this point
        self.debugger.cmd('continue')
        thread.join()
        self.debugger.control('c')
        self.debugger.cmd('q')
