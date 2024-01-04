import unittest
import pexpect
import time
import threading
import requests


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
        debugger = pexpect.spawn(
            './target/debug/bugstalker ./examples/target/debug/todos')
        debugger.expect('BugStalker greets')
        self.debugger = debugger

    # todo try uncomment this test on next rust versions
    # def test_step_over_until_response(self):
    #     """Runs a todos application and set breakpoint at http handler. Makes http request, and do `step over` command while http response not returning"""
    #     # create breakpoint
    #     self.debugger.sendline('b main.rs:108')
    #
    #     time.sleep(5)
    #     self.debugger.sendline('run')
    #     time.sleep(1)
    #
    #     event = threading.Event()
    #     thread = threading.Thread(target=send_create_todo_request,
    #                               args=(event,))
    #     thread.start()
    #     time.sleep(1)
    #
    #     # check that response returns at this point
    #     while not event.is_set():
    #         # send `step over` command otherwise
    #         self.debugger.sendline('next')
    #         self.debugger.expect("next")
    #         time.sleep(0.05)
    #
    #     self.debugger.sendline('q')

    def test_create_and_get(self):
        """Create item, then try to get it and check that it exists in debugger (by `var` command)"""
        # get breakpoint
        self.debugger.sendline('b main.rs:99')

        time.sleep(3)
        self.debugger.sendline('run')
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
        self.debugger.sendline('var locals')
        self.debugger.expect_exact(
            'todos = Vec<todos::Todo, alloc::alloc::Global> {')
        self.debugger.expect_exact('0: Todo {')
        self.debugger.expect_exact('text: String(test todo)')
        self.debugger.expect_exact('completed: bool(false)')

        self.debugger.sendline('continue')
        while not event.is_set():
            time.sleep(0.1)
        thread.join()

        self.debugger.sendline('q')
