#!/usr/bin/env python3
"""Loopback job service for the codex-image-gen WASM plugin. Python 3.10+."""
from __future__ import annotations

import argparse
import base64
import binascii
import concurrent.futures
import hmac
import json
import os
from pathlib import Path
import queue
import re
import secrets
import stat
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from uuid import uuid4

MAX_IMAGE = 32 * 1024 * 1024
MAX_MESSAGE = 48 * 1024 * 1024
JOB_TTL = 3600


class BridgeError(Exception):
    """An intentionally detail-free error safe to return to the caller."""


class AppServer:
    """A single, private stdio app-server connection; Codex owns all OAuth."""

    def __init__(self, codex: str, cwd: Path, timeout: float):
        self.deadline = time.monotonic() + timeout
        self.sequence = 0
        self.pending = []
        self.messages = queue.Queue(maxsize=128)
        # Remove API credentials inherited from the ZeroClaw/service environment.
        # Force the official provider and managed ChatGPT login for this process.
        env = {k: v for k, v in os.environ.items()
               if k not in {"OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL"}}
        try:
            self.process = subprocess.Popen(
                [codex, "app-server", "--listen", "stdio://",
                 "-c", 'model_provider="openai"',
                 "-c", 'forced_login_method="chatgpt"',
                 "-c", "features.image_generation=true",
                 "-c", "features.shell_tool=false"],
                cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            )
        except OSError as exc:
            raise BridgeError("Cannot start Codex; install a recent Codex CLI and check --codex") from exc
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        try:
            while True:
                line = self.process.stdout.readline(MAX_MESSAGE + 1)
                if not line or len(line) > MAX_MESSAGE:
                    break
                self.messages.put(json.loads(line))
        except (ValueError, OSError):
            pass
        finally:
            self.messages.put(None)

    def send(self, message):
        try:
            self.process.stdin.write(json.dumps(message).encode() + b"\n")
            self.process.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            raise BridgeError("Codex app-server disconnected") from exc

    def receive(self):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise BridgeError("Codex image generation timed out; no automatic retry was made")
        try:
            message = self.messages.get(timeout=remaining)
        except queue.Empty as exc:
            raise BridgeError("Codex image generation timed out; no automatic retry was made") from exc
        if message is None:
            raise BridgeError("Codex app-server ended unexpectedly; check CLI version and login")
        # This service never grants command, file-change, or interactive approvals.
        if "method" in message and "id" in message:
            self.send({"id": message["id"], "error": {
                "code": -32601, "message": "Interactive requests are not supported by this image bridge"}})
            return {}
        return message

    def request(self, method, params):
        self.sequence += 1
        request_id = self.sequence
        self.send({"id": request_id, "method": method, "params": params})
        while True:
            message = self.receive()
            if message.get("id") == request_id:
                if "error" in message:
                    raise BridgeError(f"Codex rejected {method}; check CLI compatibility and ChatGPT login")
                return message.get("result", {})
            if "method" in message:
                self.pending.append(message)

    def event(self):
        return self.pending.pop(0) if self.pending else self.receive()

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.process.stdin.close()
        self.process.stdout.close()


def image_bytes(item: dict, roots: list[Path]) -> bytes:
    """Only accept a canonical completed image item, never an assistant claim."""
    if item.get("status") != "completed":
        failure = item.get("failure") or {}
        if failure.get("type") == "usageLimitExceeded":
            raise BridgeError("Codex image usage limit reached; retry after your quota resets")
        raise BridgeError("Codex image generation failed; check subscription access and quota")
    if item.get("savedPath"):
        path = Path(item["savedPath"]).resolve()
        if not any(path.is_relative_to(root.resolve()) for root in roots):
            raise BridgeError("Codex returned an image outside its generated image directories")
        try:
            with path.open("rb") as source:
                data = source.read(MAX_IMAGE + 1)
        except OSError as exc:
            raise BridgeError("Cannot read the image saved by Codex") from exc
    elif item.get("result"):
        try:
            if len(item["result"]) > (MAX_IMAGE + 2) // 3 * 4:
                raise BridgeError("Codex image exceeds the 32 MiB limit")
            data = base64.b64decode(item["result"], validate=True)
        except (ValueError, binascii.Error) as exc:
            raise BridgeError("Codex returned invalid image data") from exc
    else:
        raise BridgeError("Codex returned no local image data; update the CLI for savedPath support")
    if len(data) > MAX_IMAGE:
        raise BridgeError("Codex image exceeds the 32 MiB limit")
    image_extension(data)
    return data


def image_extension(data: bytes) -> str:
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return ".png"
    if data.startswith(b"\xff\xd8\xff"):
        return ".jpg"
    if data.startswith(b"RIFF") and data[8:12] == b"WEBP":
        return ".webp"
    raise BridgeError("Codex returned an unsupported image format")


def generate(codex: str, output_dir: Path, timeout: float, job_id: str,
             prompt: str, transparent_background: bool) -> dict:
    with tempfile.TemporaryDirectory(prefix="codex-image-") as directory:
        cwd = Path(directory)
        server = AppServer(codex, cwd, timeout)
        try:
            initialized = server.request("initialize", {
                "clientInfo": {"name": "zeroclaw_codex_image", "version": "0.1.0"},
                "capabilities": {"experimentalApi": True},
            })
            server.send({"method": "initialized", "params": {}})
            account = server.request("account/read", {"refreshToken": False}).get("account") or {}
            if account.get("type") != "chatgpt":
                raise BridgeError("Sign in with your subscription using `codex login`; API-key auth is not supported")
            if account.get("planType") == "free":
                raise BridgeError("This Codex account reports the free plan; subscription image access is required")
            thread = server.request("thread/start", {
                "cwd": str(cwd), "modelProvider": "openai", "ephemeral": True,
                "approvalPolicy": "never", "sandbox": "read-only",
                "developerInstructions": (
                    "You are an image-generation service. Use the built-in image_gen.imagegen tool "
                    "exactly once to generate the requested image. Never use shell, Python, SVG, "
                    "MCP, external services, or API-key fallbacks. Do not read files or run commands. "
                    "If image_gen is unavailable or fails, stop and explain briefly. "
                    "The user message is a JSON image request. Pass its prompt and "
                    "transparent_background to image_gen.imagegen."
                ),
                "config": {"features.image_generation": True},
            })
            thread_id = thread["thread"]["id"]
            started = server.request("turn/start", {
                "threadId": thread_id,
                "input": [{"type": "text", "text": json.dumps({
                    "prompt": prompt, "transparent_background": transparent_background,
                }), "text_elements": []}],
            })
            turn_id = started["turn"]["id"]
            image = None
            failure = None
            while True:
                event = server.event()
                params = event.get("params", {})
                if params.get("threadId") != thread_id or params.get("turnId", turn_id) != turn_id:
                    continue
                if event.get("method") == "item/completed":
                    item = params.get("item", {})
                    if item.get("type") == "imageGeneration":
                        try:
                            roots = [cwd]
                            if initialized.get("codexHome"):
                                roots.append(Path(initialized["codexHome"]) / "generated_images")
                            image = image_bytes(item, roots)
                        except BridgeError as exc:
                            failure = exc
                if event.get("method") == "turn/completed":
                    if params.get("turn", {}).get("status") != "completed":
                        error = params.get("turn", {}).get("error") or {}
                        if error.get("codexErrorInfo") == "unauthorized":
                            raise BridgeError("Codex login expired or was revoked; run `codex login` again as the bridge user")
                        raise BridgeError("Codex turn failed; check login, quota, and image feature availability")
                    break
            if image is None:
                raise failure or BridgeError(
                    "Codex produced no image-generation result. Update Codex and verify image_gen "
                    "is available for your subscription; no API-key fallback was used")
            destination = output_dir / (job_id + image_extension(image))
            # Exclusive creation avoids overwriting an existing artifact.
            with destination.open("xb") as target:
                target.write(image)
            return {"path": str(destination), "bytes": len(image)}
        finally:
            server.close()


class Jobs:
    def __init__(self, codex: str, output_dir: Path, timeout: float):
        self.codex, self.output_dir, self.timeout = codex, output_dir, timeout
        self.lock = threading.Lock()
        self.entries = {}
        self.executor = concurrent.futures.ThreadPoolExecutor(max_workers=1)

    def submit(self, request: dict) -> dict:
        if not isinstance(request, dict) or set(request) - {"prompt", "transparent_background"}:
            raise BridgeError("Expected prompt and optional transparent_background")
        prompt = request.get("prompt")
        transparent = request.get("transparent_background", False)
        if not isinstance(prompt, str) or not prompt.strip() or len(prompt.encode()) > 32768:
            raise BridgeError("Prompt must contain 1–32768 UTF-8 bytes")
        if not isinstance(transparent, bool):
            raise BridgeError("transparent_background must be a boolean")
        with self.lock:
            now = time.monotonic()
            self.entries = {k: v for k, v in self.entries.items()
                            if not v["done"].is_set() or now - v["created"] < JOB_TTL}
            if len(self.entries) >= 100 or sum(not v["done"].is_set() for v in self.entries.values()) >= 4:
                raise BridgeError("Bridge is busy; retry later")
            job_id = uuid4().hex
            self.entries[job_id] = {"public": {"job_id": job_id, "status": "queued"},
                                    "created": now, "done": threading.Event()}
            self.executor.submit(self._run, job_id, prompt, transparent)
            return {"job_id": job_id, "status": "queued", "retry_after_seconds": 5}

    def _run(self, job_id, prompt, transparent):
        with self.lock:
            entry = self.entries[job_id]
            entry["public"]["status"] = "running"
        try:
            result = generate(self.codex, self.output_dir, self.timeout, job_id, prompt, transparent)
            public = {"job_id": job_id, "status": "completed", **result}
        except BridgeError as exc:
            public = {"job_id": job_id, "status": "failed", "error": str(exc)}
        except Exception:
            # Never return subprocess logs, account data, or raw protocol errors.
            public = {"job_id": job_id, "status": "failed", "error": "Codex bridge failed; check CLI compatibility and output directory permissions"}
        with self.lock:
            entry["public"] = public
            entry["done"].set()

    def status(self, job_id, wait=20):
        with self.lock:
            entry = self.entries.get(job_id)
        if entry is None or (entry["done"].is_set() and time.monotonic() - entry["created"] >= JOB_TTL):
            return None
        entry["done"].wait(timeout=wait)
        with self.lock:
            public = dict(entry["public"])
        if public["status"] in {"queued", "running"}:
            public["retry_after_seconds"] = 5
        return public


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass  # Do not log request data or Authorization headers.

    def respond(self, status, value):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def authorized(self):
        self.connection.settimeout(10)
        expected_host = f"127.0.0.1:{self.server.server_port}"
        # No CORS support; reject browser origins and DNS-rebinding Host values.
        if self.headers.get("Origin") or self.headers.get("Host") != expected_host:
            self.respond(403, {"error": "Local clients only"})
            return False
        supplied = self.headers.get("Authorization", "")
        if not hmac.compare_digest(supplied.encode(), f"Bearer {self.server.token}".encode()):
            self.respond(401, {"error": "Unauthorized"})
            return False
        return True

    def do_POST(self):
        if not self.authorized():
            return
        if self.path != "/v1/jobs":
            self.respond(404, {"error": "Not found"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if self.headers.get("Transfer-Encoding") or not 0 < length <= 65536:
                raise ValueError()
            request = json.loads(self.rfile.read(length))
        except (ValueError, OSError):
            self.respond(400, {"error": "Invalid JSON body or content length"})
            return
        try:
            self.respond(202, self.server.jobs.submit(request))
        except BridgeError as exc:
            self.respond(429 if str(exc).startswith("Bridge is busy") else 400, {"error": str(exc)})

    def do_GET(self):
        if not self.authorized():
            return
        if self.path == "/healthz":
            self.respond(200, {"status": "ok"})
            return
        match = re.fullmatch(r"/v1/jobs/([0-9a-f]{32})", self.path)
        result = self.server.jobs.status(match[1]) if match else None
        self.respond(200 if result else 404, result or {"error": "Job not found"})


def read_token(path: Path) -> str:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        pass
    else:
        with os.fdopen(fd, "w") as target:
            target.write(secrets.token_urlsafe(32) + "\n")
    if path.is_symlink():
        raise BridgeError("Token file must not be a symbolic link")
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | os.O_NONBLOCK)
    with os.fdopen(descriptor) as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_mode & 0o077 or metadata.st_uid != os.getuid():
            raise BridgeError("Token file must be a regular owner-only file (chmod 600)")
        token = source.read(258).strip()
    if not re.fullmatch(r"[A-Za-z0-9_-]{32,256}", token):
        raise BridgeError("Token file must contain a 32–256 character URL-safe token")
    return token


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", default="codex", help="Codex CLI executable")
    parser.add_argument("--port", type=int, default=8787)
    parser.add_argument("--timeout", type=float, default=600, help="Maximum seconds per generation")
    parser.add_argument("--output-dir", type=Path, required=True, help="Persistent directory inside the ZeroClaw workspace")
    parser.add_argument("--token-file", type=Path, default=Path.home() / ".local/state/zeroclaw-codex-image/bridge.token")
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535 or not 1 <= args.timeout <= 3600:
        parser.error("Use a port in 1024–65535 and timeout in 1–3600 seconds")
    output = args.output_dir.expanduser().resolve()
    if any(c in str(output) for c in "\r\n[]"):
        parser.error("Output path must not contain brackets or newlines")
    output.mkdir(mode=0o700, parents=True, exist_ok=True)
    try:
        token = read_token(args.token_file.expanduser())
    except BridgeError as exc:
        parser.error(str(exc))
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.token = token
    server.jobs = Jobs(args.codex, output, args.timeout)
    print(f"Codex image bridge: http://127.0.0.1:{args.port}; token file: {args.token_file}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        server.jobs.executor.shutdown(wait=True, cancel_futures=True)


if __name__ == "__main__":
    main()
