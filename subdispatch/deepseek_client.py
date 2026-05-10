from __future__ import annotations

from .provider import ProviderClient, ProviderError, SYSTEM_PROMPT


class DeepSeekError(ProviderError):
    pass


class DeepSeekClient(ProviderClient):
    """Backward-compatible alias for the default DeepSeek provider."""

    def __init__(self, *args, **kwargs):
        kwargs.setdefault("provider", "deepseek")
        try:
            super().__init__(*args, **kwargs)
        except ProviderError as e:
            raise DeepSeekError(str(e)) from e
