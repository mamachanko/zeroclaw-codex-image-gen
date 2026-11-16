import base64
import http.client
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

import codex_image_bridge as bridge

PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1cAAAAASUVORK5CYII=")


class FakeAppServer:
    account = {"type": "chatgpt", "planType": "plus"}
    item = {"type": "imageGeneration", "status": "completed", "result": base64.b64encode(PNG).decode()}
    turn_status = "completed"
    hang = False
    seen = []

    def __init__(self, *args):
        self.events = iter([
            {"method": "item/completed", "params": {"threadId": "other", "turnId": "turn", "item": self.item}},
            {"method": "item/completed", "params": {"threadId": "thread", "turnId": "turn", "item": self.item}},
            {"method": "turn/completed", "params": {"threadId": "thread", "turn": {"id": "turn", "status": self.turn_status}}},
        ])

    def request(self, method, params):
        self.seen.append((method, params))
        return {"initialize": {}, "account/read": {"account": self.account},
                "thread/start": {"thread": {"id": "thread"}}, "turn/start": {"turn": {"id": "turn"}}}[method]

    def send(self, msg):
        pass

    def event(self):
        return next(self.events)

    def close(self):
        pass


class GenerationTests(unittest.TestCase):
    def test_image_event_is_saved_and_prompt_is_data(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(bridge, "AppServer", FakeAppServer):
            result = bridge.generate("codex", Path(directory), 10, "a" * 32, "fox $(id) `id`", True)
            self.assertEqual(Path(result["path"]).read_bytes(), PNG)
            turn = [params for method, params in FakeAppServer.seen if method == "turn/start"][-1]
            self.assertEqual(json.loads(turn["input"][0]["text"])["prompt"], "fox $(id) `id`")
            thread = [params for method, params in FakeAppServer.seen if method == "thread/start"][-1]
            self.assertEqual(thread["sandbox"], "read-only")

    def test_api_key_and_free_accounts_fail_before_generation(self):
        for account in [{"type": "apiKey"}, {"type": "chatgpt", "planType": "free"}, {}]:
            with self.subTest(account=account), tempfile.TemporaryDirectory() as directory:
                with patch.object(bridge, "AppServer", FakeAppServer), patch.object(FakeAppServer, "account", account):
                    with self.assertRaises(bridge.BridgeError):
                        bridge.generate("codex", Path(directory), 10, "a" * 32, "fox", False)
                self.assertEqual(list(Path(directory).iterdir()), [])

    def test_assistant_claim_is_not_an_image(self):
        item = {"type": "agentMessage", "text": "Image saved at /tmp/fake.png"}
        with tempfile.TemporaryDirectory() as directory, patch.object(bridge, "AppServer", FakeAppServer), patch.object(FakeAppServer, "item", item):
            with self.assertRaisesRegex(bridge.BridgeError, "no image-generation result"):
                bridge.generate("codex", Path(directory), 10, "a" * 32, "fox", False)

    def test_failed_turn_and_quota_do_not_return_success(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(bridge, "AppServer", FakeAppServer):
            with patch.object(FakeAppServer, "turn_status", "failed"), self.assertRaisesRegex(bridge.BridgeError, "turn failed"):
                bridge.generate("codex", Path(directory), 10, "a" * 32, "fox", False)
            with patch.object(FakeAppServer, "item", {"type": "imageGeneration", "status": "failed", "failure": {"type": "usageLimitExceeded"}}):
                with self.assertRaisesRegex(bridge.BridgeError, "usage limit"):
                    bridge.generate("codex", Path(directory), 10, "a" * 32, "fox", False)

    def test_saved_paths_are_confined_and_image_data_is_bounded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            image = root / "image.png"
            image.write_bytes(PNG)
            self.assertEqual(bridge.image_bytes({"status": "completed", "savedPath": str(image)}, [root]), PNG)
            with self.assertRaisesRegex(bridge.BridgeError, "outside"):
                bridge.image_bytes({"status": "completed", "savedPath": "/etc/passwd"}, [root])
            for data in ["!!!", base64.b64encode(b"not an image").decode()]:
                with self.assertRaises(bridge.BridgeError):
                    bridge.image_bytes({"status": "completed", "result": data}, [root])
            with patch.object(bridge, "MAX_IMAGE", 8), self.assertRaisesRegex(bridge.BridgeError, "32 MiB"):
                bridge.image_bytes({"status": "completed", "savedPath": str(image)}, [root])


class HTTPTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.jobs = bridge.Jobs("codex", Path(self.directory.name), 10)
        self.server = bridge.ThreadingHTTPServer(("127.0.0.1", 0), bridge.Handler)
        self.server.token = "a" * 43
        self.server.jobs = self.jobs
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.jobs.executor.shutdown()
        self.directory.cleanup()

    def request(self, method, path, body=None, headers=None):
        conn = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=10)
        supplied = {"Authorization": "Bearer " + self.server.token}
        supplied.update(headers or {})
        conn.request(method, path, body=body, headers=supplied)
        response = conn.getresponse()
        result = response.status, json.loads(response.read())
        conn.close()
        return result

    def test_authenticated_generation_to_status(self):
        with patch.object(bridge, "AppServer", FakeAppServer):
            status, job = self.request("POST", "/v1/jobs", json.dumps({"prompt": "fox"}))
            self.assertEqual(status, 202)
            status, result = self.request("GET", "/v1/jobs/" + job["job_id"])
            self.assertEqual(status, 200)
            self.assertEqual(result["status"], "completed")
            self.assertEqual(Path(result["path"]).read_bytes(), PNG)

    def test_auth_and_browser_origin_rejection(self):
        self.assertEqual(self.request("GET", "/healthz", headers={"Authorization": "Bearer wrong"})[0], 401)
        self.assertEqual(self.request("GET", "/healthz", headers={"Origin": "https://example.org"})[0], 403)
        self.assertEqual(self.request("GET", "/healthz", headers={"Host": "example.org"})[0], 403)

    def test_malformed_input_and_unknown_jobs(self):
        for body in ["not json", "[]", '{"prompt":""}', '{"prompt":"fox","api_key":"secret"}']:
            self.assertEqual(self.request("POST", "/v1/jobs", body)[0], 400)
        self.assertEqual(self.request("GET", "/v1/jobs/" + "a" * 32)[0], 404)

    def test_job_polling_does_not_resubmit(self):
        started, release = threading.Event(), threading.Event()

        def slow_generate(*_args):
            started.set()
            release.wait(5)
            return {"path": "/images/test.png"}

        with patch.object(bridge, "generate", side_effect=slow_generate) as generate:
            job = self.jobs.submit({"prompt": "fox"})
            self.assertTrue(started.wait(2))
            try:
                for _ in range(3):
                    self.assertEqual(self.jobs.status(job["job_id"], wait=0)["status"], "running")
                self.assertEqual(generate.call_count, 1)
            finally:
                release.set()


class TokenTests(unittest.TestCase):
    def test_creates_private_token_and_reuses_it(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "token"
            token = bridge.read_token(path)
            self.assertEqual(bridge.read_token(path), token)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            path.chmod(0o644)
            with self.assertRaises(bridge.BridgeError):
                bridge.read_token(path)


class SubprocessTests(unittest.TestCase):
    def test_real_stdio_transport_buffers_events_and_removes_api_credentials(self):
        # A separate process exercises JSONL framing and the actual command/env,
        # without contacting an account or spending image quota.
        script = '''#!/usr/bin/env python3
import json, os, sys
assert "OPENAI_API_KEY" not in os.environ
assert "CODEX_API_KEY" not in os.environ
assert "OPENAI_BASE_URL" not in os.environ
def emit(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    msg = json.loads(line)
    if "method" not in msg:
        assert msg["id"] == 999
        assert msg["error"]["code"] == -32601
        continue
    method = msg["method"]
    if method == "initialized": continue
    result = {}
    if method == "account/read": result = {"account":{"type":"chatgpt","planType":"plus"}}
    if method == "thread/start": result = {"thread":{"id":"t"}}
    if method == "turn/start":
        emit({"id":999,"method":"item/commandExecution/requestApproval","params":{}})
        emit({"method":"item/completed","params":{"threadId":"t","turnId":"u","item":{
            "type":"imageGeneration","status":"completed","result":"IMAGE_PLACEHOLDER"}}})
        emit({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed"}}})
        result = {"turn":{"id":"u"}}
    emit({"id":msg["id"],"result":result})
'''.replace("IMAGE_PLACEHOLDER", base64.b64encode(PNG).decode())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = root / "fake-codex"
            executable.write_text(script)
            executable.chmod(0o700)
            with patch.dict(os.environ, {"OPENAI_API_KEY": "test-key", "CODEX_API_KEY": "test-key", "OPENAI_BASE_URL": "https://example.invalid"}):
                result = bridge.generate(str(executable), root, 10, "a" * 32, "a fox", False)
            self.assertEqual(Path(result["path"]).read_bytes(), PNG)

    def test_timeout_terminates_process_and_disconnect_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = root / "fake-codex"
            executable.write_text("#!/usr/bin/env python3\nimport time\ntime.sleep(60)\n")
            executable.chmod(0o700)
            server = bridge.AppServer(str(executable), root, 0.05)
            try:
                with self.assertRaisesRegex(bridge.BridgeError, "timed out"):
                    server.request("initialize", {})
            finally:
                server.close()
            self.assertIsNotNone(server.process.poll())
            executable.write_text("#!/usr/bin/env python3\n")
            server = bridge.AppServer(str(executable), root, 5)
            try:
                with self.assertRaises(bridge.BridgeError):
                    server.request("initialize", {})
            finally:
                server.close()


if __name__ == "__main__":
    unittest.main()
