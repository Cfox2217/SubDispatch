"""SubDispatch: local parallel child-agent scaffold."""
from __future__ import annotations

__version__ = "0.2.0"

from . import config
from . import context
from . import cost
from . import deepseek_client
from . import engine
from . import installer
from . import provider
from . import router
from . import schema
from . import subdispatch
from . import verifier
from .config import PROVIDER_PRESETS, normalize_provider, save_provider_config
from .engine import SubDispatchDelegateEngine
from .subdispatch import SubDispatchEngine, WorkerConfig, init_env

__all__ = [
    "__version__",
    "config",
    "context",
    "cost",
    "deepseek_client",
    "engine",
    "installer",
    "provider",
    "router",
    "schema",
    "subdispatch",
    "verifier",
    "PROVIDER_PRESETS",
    "normalize_provider",
    "save_provider_config",
    "SubDispatchDelegateEngine",
    "SubDispatchEngine",
    "WorkerConfig",
    "init_env",
]
