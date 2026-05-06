from __future__ import annotations

from pathlib import Path
from typing import List

from .schema import FileContext


class ContextPacker:
    """Loads bounded file context for DeepSeek."""

    def __init__(self, max_files: int = 8, max_chars_per_file: int = 24_000,
                 max_total_chars: int = 120_000):
        self.max_files = max_files
        self.max_chars_per_file = max_chars_per_file
        self.max_total_chars = max_total_chars

    def load(self, file_paths: List[str]) -> List[FileContext]:
        contexts: List[FileContext] = []
        total = 0
        for raw in file_paths[: self.max_files]:
            path = Path(raw)
            if not path.exists() or not path.is_file():
                contexts.append(FileContext(path=raw, content=f"/* CodexSaver: file not found: {raw} */"))
                continue
            content = path.read_text(encoding="utf-8", errors="replace")
            if len(content) > self.max_chars_per_file:
                content = content[: self.max_chars_per_file] + "\n\n/* ... truncated by CodexSaver ... */"
            remaining = self.max_total_chars - total
            if remaining <= 0:
                break
            if len(content) > remaining:
                content = content[:remaining] + "\n\n/* ... total context truncated by CodexSaver ... */"
            contexts.append(FileContext(path=str(path), content=content))
            total += len(content)
        return contexts
