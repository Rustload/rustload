"""Writes `latest.json`, the update manifest of the desktop app, next to the release files.

Usage: update_manifest.py <dist dir> <version> <owner/repo>

The app reads it from releases/latest/download/latest.json and only installs a file whose
signature (`<file>.sig`, made with the project's updater key) checks out.
"""

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

dist, version, repository = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
base = f"https://github.com/{repository}/releases/download/v{version}"


def entry(name: str) -> dict:
    file, signature = dist / name, dist / f"{name}.sig"
    if not file.is_file() or not signature.is_file():
        sys.exit(f"missing {name} or its signature")
    return {"url": f"{base}/{name}", "signature": signature.read_text(encoding="utf-8").strip()}


mac = entry(f"Rustload_{version}_universal.app.tar.gz")
manifest = {
    "version": version,
    "notes": f"https://github.com/{repository}/releases/tag/v{version}",
    "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {
        "windows-x86_64": entry(f"Rustload_{version}_x64-setup.exe"),
        "darwin-aarch64": mac,
        "darwin-x86_64": mac,
        "linux-x86_64": entry(f"Rustload_{version}_amd64.AppImage"),
    },
}
(dist / "latest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print(json.dumps(manifest, indent=2))
