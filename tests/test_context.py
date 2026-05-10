from __future__ import annotations

import os
import pytest
import tempfile
from subdispatch.context import ContextPacker


class TestContextPacker:
    def setup_method(self):
        self.packer = ContextPacker()

    def test_load_single_file(self):
        with tempfile.NamedTemporaryFile(mode="w", suffix=".py", delete=False) as f:
            f.write("def foo(): return 42\n")
            path = f.name
        try:
            contexts = self.packer.load([path])
            assert len(contexts) == 1
            assert "def foo" in contexts[0].content
            assert contexts[0].path.endswith(".py")
        finally:
            os.unlink(path)

    def test_load_nonexistent_file(self):
        contexts = self.packer.load(["nonexistent/file.py"])
        assert len(contexts) == 1
        assert "not found" in contexts[0].content

    def test_load_respects_max_files(self):
        packer = ContextPacker(max_files=2)
        with tempfile.TemporaryDirectory() as tmpdir:
            for i in range(5):
                with open(os.path.join(tmpdir, f"f{i}.txt"), "w") as f:
                    f.write(f"content {i}")
            contexts = packer.load([os.path.join(tmpdir, f"f{i}.txt") for i in range(5)])
            assert len(contexts) == 2

    def test_load_truncates_large_file(self):
        packer = ContextPacker(max_chars_per_file=50)
        with tempfile.NamedTemporaryFile(mode="w", suffix=".py", delete=False) as f:
            f.write("x" * 200)
            path = f.name
        try:
            contexts = packer.load([path])
            assert len(contexts[0].content) < 200
            assert "truncated" in contexts[0].content
        finally:
            os.unlink(path)

    def test_load_respects_total_chars(self):
        packer = ContextPacker(max_total_chars=100)
        with tempfile.TemporaryDirectory() as tmpdir:
            for i in range(3):
                with open(os.path.join(tmpdir, f"f{i}.txt"), "w") as f:
                    f.write("x" * 60)
            contexts = packer.load([os.path.join(tmpdir, f"f{i}.txt") for i in range(3)])
            total = sum(len(c.content) for c in contexts)
            assert total <= 100 + 60  # last file may partially fill before truncating

    def test_load_empty_list(self):
        contexts = self.packer.load([])
        assert len(contexts) == 0

    def test_load_resolves_relative_path_from_workspace(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            nested = os.path.join(tmpdir, "src")
            os.makedirs(nested)
            file_path = os.path.join(nested, "example.py")
            with open(file_path, "w") as f:
                f.write("print('ok')\n")
            packer = ContextPacker(workspace=tmpdir)
            contexts = packer.load(["src/example.py"])
            assert len(contexts) == 1
            assert contexts[0].path == os.path.realpath(file_path)
            assert "print('ok')" in contexts[0].content
