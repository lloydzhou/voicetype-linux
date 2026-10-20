"""Small, offline integration tests for the installed model-download command."""

import http.server
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "download-model.sh"
REPO = "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
MODEL = b"\x08\x01fake ONNX model\x00\xff"
TOKENS = b"<blk> 0\nhello 1\n"


class DownloadModelTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="voicetype download ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "models with spaces"
        self.responses = {
            "model.int8.onnx": (200, MODEL),
            "tokens.txt": (200, TOKENS),
        }
        self.requests = []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                owner.requests.append(self.path)
                prefix = f"/{REPO}/resolve/main/"
                if self.path.startswith(prefix):
                    status, data = owner.responses.get(self.path[len(prefix):], (404, b"missing"))
                else:
                    status, data = 404, b"unexpected path"
                self.send_response(status)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.stop_server)
        self.base = f"http://127.0.0.1:{self.server.server_port}"
        self.env = {
            key: value for key, value in os.environ.items()
            if key.lower() not in {"http_proxy", "https_proxy", "all_proxy", "no_proxy"}
        }
        self.env.update(HOME=str(self.root), HF_ENDPOINT=self.base, NO_PROXY="127.0.0.1")
        self.env.pop("XDG_DATA_HOME", None)

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def run_script(self, *args):
        return subprocess.run(
            ["bash", str(SCRIPT), *args], env=self.env, cwd=self.root,
            capture_output=True, text=True, timeout=15,
        )

    def existing_files(self):
        self.output.mkdir()
        (self.output / "model.int8.onnx").write_bytes(b"old model")
        (self.output / "tokens.txt").write_bytes(b"old tokens")
        (self.output / "unrelated").mkdir()
        (self.output / "unrelated" / "keep").write_bytes(b"keep")

    def assert_existing_untouched(self):
        self.assertEqual((self.output / "model.int8.onnx").read_bytes(), b"old model")
        self.assertEqual((self.output / "tokens.txt").read_bytes(), b"old tokens")
        self.assertEqual((self.output / "unrelated" / "keep").read_bytes(), b"keep")
        self.assertEqual(list(self.output.glob(".voicetype-download.*")), [])

    def test_success_replaces_pair_and_keeps_unknown_contents(self):
        self.existing_files()
        result = self.run_script("--hf-base", self.base + "/", "--output-dir", str(self.output))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.output / "model.int8.onnx").read_bytes(), MODEL)
        self.assertEqual((self.output / "tokens.txt").read_bytes(), TOKENS)
        self.assertEqual((self.output / "unrelated" / "keep").read_bytes(), b"keep")
        self.assertEqual(list(self.output.glob(".voicetype-download.*")), [])
        self.assertEqual(self.requests, [f"/{REPO}/resolve/main/{name}" for name in self.responses])

    def test_failed_second_download_retains_existing_pair(self):
        self.existing_files()
        self.responses["tokens.txt"] = (404, b"not found")
        result = self.run_script("--output-dir", str(self.output))
        self.assertNotEqual(result.returncode, 0)
        self.assert_existing_untouched()

    def test_failed_first_download_retains_existing_pair(self):
        self.existing_files()
        self.responses["model.int8.onnx"] = (403, b"forbidden")
        result = self.run_script("--output-dir", str(self.output))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.requests), 1)
        self.assert_existing_untouched()

    def test_invalid_download_retains_existing_pair(self):
        self.existing_files()
        for name in self.responses:
            for body in (b"", b" \n<!DOCTYPE HTML><html>error</html>", b"<HTML lang='en'>error"):
                with self.subTest(name=name, body=body):
                    self.responses = {"model.int8.onnx": (200, MODEL), "tokens.txt": (200, TOKENS)}
                    self.responses[name] = (200, body)
                    result = self.run_script("--output-dir", str(self.output))
                    self.assertNotEqual(result.returncode, 0)
                    self.assert_existing_untouched()

    def test_default_home_and_hf_endpoint(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        target = self.root / ".local/share/voicetype/models/sensevoice"
        self.assertEqual((target / "model.int8.onnx").read_bytes(), MODEL)
        self.assertEqual((target / "tokens.txt").read_bytes(), TOKENS)

    def test_default_xdg_data_home(self):
        self.env["XDG_DATA_HOME"] = str(self.root / "xdg data")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        target = self.root / "xdg data/voicetype/models/sensevoice"
        self.assertEqual((target / "tokens.txt").read_bytes(), TOKENS)

    def test_relative_output_path(self):
        result = self.run_script("--output-dir", "relative directory")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / "relative directory/tokens.txt").read_bytes(), TOKENS)

    def test_help_needs_no_home_or_network(self):
        self.env.pop("HOME")
        result = self.run_script("--help")
        self.assertEqual(result.returncode, 0)
        self.assertIn("voicetype-download-model", result.stdout)
        self.assertIn("not an atomic pair", result.stdout)
        self.assertEqual(self.requests, [])

    def test_argument_validation_does_not_use_network_or_echo_credentials(self):
        for args in (
            ["--unknown"], ["--output-dir"], ["--hf-base"], ["--proxy"],
            ["--output-dir", ""], ["--hf-base", "--output-dir"],
            ["--hf-base", "file:///etc/passwd"], ["--hf-base", "https://"],
            ["--hf-base", "https://example.test/?query"],
            ["--proxy", "bad://user:secret@example.test"],
            ["--output-dir", "/"],
        ):
            with self.subTest(args=args):
                result = self.run_script(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("secret", result.stdout + result.stderr)
        self.assertEqual(self.requests, [])

    def test_directory_destination_is_rejected_without_touching_files(self):
        self.existing_files()
        (self.output / "tokens.txt").unlink()
        (self.output / "tokens.txt").mkdir()
        result = self.run_script("--output-dir", str(self.output))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.output / "model.int8.onnx").read_bytes(), b"old model")
        self.assertEqual(self.requests, [])

    def test_proxy_environment_and_explicit_proxy(self):
        # The local server serves as an HTTP proxy: inspect the absolute request
        # URL and intentionally return 404, without resolving the fake endpoint.
        self.env["NO_PROXY"] = ""
        self.env["http_proxy"] = self.base
        endpoint = "http://not-a-real-host.invalid"
        result = self.run_script("--hf-base", endpoint, "--output-dir", str(self.output))
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.requests[-1].startswith(endpoint + "/"))
        self.env["http_proxy"] = "http://127.0.0.1:1"
        explicit_proxy = self.base.replace("http://", "http://user:secret@")
        result = self.run_script("--hf-base", endpoint, "--proxy", explicit_proxy,
                                 "--output-dir", str(self.output))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.requests), 2)
        self.assertNotIn("secret", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
