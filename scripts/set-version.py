"""Writes the release version everywhere: python3 scripts/set-version.py v1.2 → 1.2.0.

Cargo.toml ([workspace.package] version), Cargo.lock (the workspace crates) and web/package.json.
"""
import json, pathlib, re, sys

root = pathlib.Path(__file__).resolve().parent.parent
raw = sys.argv[1].strip().lstrip("v")
parts = raw.split("-")[0].split(".")
if not 1 <= len(parts) <= 3 or not all(p.isdigit() for p in parts):
    sys.exit(f"not a version: {sys.argv[1]}")
version = ".".join((parts + ["0", "0"])[:3])

cargo = root / "Cargo.toml"
text = cargo.read_text()
text, n = re.subn(r'(\[workspace\.package\][^\[]*?\nversion = )"[^"]*"', rf'\g<1>"{version}"', text, count=1, flags=re.S)
if n != 1:
    sys.exit("no [workspace.package] version in Cargo.toml")
cargo.write_text(text)

crates = {p.parent.name for p in (root / "crates").glob("*/Cargo.toml")}
names = set()
for p in (root / "crates").glob("*/Cargo.toml"):
    m = re.search(r'^name = "([^"]+)"', p.read_text(), re.M)
    if m:
        names.add(m.group(1))
lock = root / "Cargo.lock"
if lock.exists():
    def bump(m):
        return f'name = "{m.group(1)}"\nversion = "{version}"' if m.group(1) in names else m.group(0)
    lock.write_text(re.sub(r'name = "([^"]+)"\nversion = "[^"]*"', bump, lock.read_text()))

pkg = root / "web" / "package.json"
data = json.loads(pkg.read_text())
data["version"] = version
pkg.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
lockfile = root / "web" / "package-lock.json"
if lockfile.exists():
    data = json.loads(lockfile.read_text())
    data["version"] = version
    if "" in data.get("packages", {}):
        data["packages"][""]["version"] = version
    lockfile.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
print(version)
