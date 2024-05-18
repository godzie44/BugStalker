import unittest
import time
import threading
import requests
from helper import Debugger


def send_create_todo_request(event):
    url = "http://localhost:3000/todos"
    requests.post(url, json={"text": "test todo"})
    event.set()


def send_get_todo_request(event):
    url = "http://localhost:3000/todos"
    requests.get(url)
    event.set()


class TodosTestCase(unittest.TestCase):
    """Test debugger on application from axum framework examples"""

    def setUp(self):
        self.debugger = Debugger('./examples/target/debug/todos')

    def test_step_over_until_response(self):
        """Runs a todos application and set breakpoint at http handler.
        Makes http request, and do `step over`
        command while http response is not returning"""
        # create breakpoint
        self.debugger.cmd('b main.rs:108')

        time.sleep(5)
        self.debugger.cmd('run')
        time.sleep(1)

        event = threading.Event()
        thread = threading.Thread(target=send_create_todo_request,
                                  args=(event,))
        thread.start()
        time.sleep(1)

        # check that response returns at this point
        while not event.is_set():
            # send `step over` command otherwise
            self.debugger.cmd('next', 'next')
            time.sleep(0.05)

        self.debugger.cmd('q')

    def test_create_and_get(self):
        """Create an item, then try to get it and check that it exists in debugger (by `var` command)"""
        # get breakpoint
        self.debugger.cmd('b main.rs:99')

        time.sleep(3)
        self.debugger.cmd('run')
        time.sleep(3)

        event = threading.Event()
        thread = threading.Thread(target=send_create_todo_request,
                                  args=(event,))
        thread.start()
        time.sleep(1)

        event = threading.Event()
        thread = threading.Thread(target=send_get_todo_request, args=(event,))
        thread.start()
        time.sleep(1)

        # break in get
        self.debugger.cmd(
            'var locals',
            'todos = Vec<todos::Todo, alloc::alloc::Global> {',
            '0: Todo {',
            'text: String(test todo)',
            'completed: bool(false)',
        )

        self.debugger.cmd('continue')
        while not event.is_set():
            time.sleep(0.1)
        thread.join()

        self.debugger.cmd('q')
